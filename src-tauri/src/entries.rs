//! Entry DTOs for the frontend and the payload operations behind the entry commands.
//!
//! Contract (PLAN.md §3.5): `list` returns metadata only, `detail` never contains the
//! password or secret custom field values, and `save` takes `password: None` to keep
//! the current one. Secrets reach the WebView only through explicit reveal (T7).

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use xpass_core::model::{CustomField, Entry, HistoryItem, Payload};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{AppError, AppResult};

/// Row in the entry list: metadata only, no secrets.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: Uuid,
    pub title: String,
    pub username: String,
    /// Host of the first URL, for display and search.
    pub url_host: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub updated_at: i64,
}

/// Entry for the detail view and the edit form, without the password and without
/// the values of secret custom fields.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    pub id: Uuid,
    pub title: String,
    pub username: String,
    pub has_password: bool,
    pub urls: Vec<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub fields: Vec<FieldView>,
    pub history_count: usize,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub name: String,
    /// `None` for secret fields.
    pub value: Option<String>,
    pub secret: bool,
}

/// Custom fields per entry, and the size of a name and a value (v0.8 plan V6).
pub const MAX_FIELDS: usize = 50;
pub const MAX_FIELD_NAME: usize = 100;
pub const MAX_FIELD_VALUE: usize = 10_000;

/// Create (`id: None`) or update an entry. `fields: None` keeps the stored custom
/// fields; `Some` replaces them in that order (v0.8 plan V6).
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct EntryInput {
    #[zeroize(skip)]
    pub id: Option<Uuid>,
    pub title: String,
    #[serde(default)]
    pub username: String,
    /// `None` keeps the current password; `Some` replaces it (empty clears it).
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub fields: Option<Vec<FieldInput>>,
}

/// One custom field of the edit form.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct FieldInput {
    pub name: String,
    #[serde(default)]
    pub secret: bool,
    pub value: FieldValue,
}

/// A new value, or the stored value of the field that was at `index` when the
/// form was loaded. `name` must still match, so a list changed under the form (a
/// sync merge) gives `not_found` instead of another field's value. Secret values
/// never have to reach the WebView to keep them.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub enum FieldValue {
    Set(String),
    Keep { index: usize, name: String },
}

/// No field values: `Debug` output must never carry a password or notes.
impl std::fmt::Debug for EntryInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryInput")
            .field("id", &self.id)
            .field("password_set", &self.password.is_some())
            .field("fields", &self.fields.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// A copyable/revealable part of an entry.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldRef {
    Username,
    Password,
    Url {
        index: usize,
    },
    Field {
        index: usize,
    },
    /// Password of the history item at `index` (oldest first). `at` must match the
    /// item's time, so a list changed under the UI (sync merge) gives `not_found`
    /// instead of another password.
    History {
        index: usize,
        at: i64,
    },
}

/// One password history item for the entry view, without the password.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    /// Position in the stored history (oldest first); with `at`, the [`FieldRef`].
    pub index: usize,
    /// Unix ms when this password was replaced.
    pub at: i64,
    /// Only when it differs from the entry's current username.
    pub username: Option<String>,
    /// `conflict`, `restore` or `import` (D1, S14, I5).
    pub note: Option<String>,
    pub has_password: bool,
}

/// Value to copy. Empty values are rejected so a copy never just clears the clipboard.
pub fn field_value(payload: &Payload, id: Uuid, field: FieldRef) -> AppResult<Zeroizing<String>> {
    let e = find(payload, id)?;
    let value = match field {
        FieldRef::Username => &e.username,
        FieldRef::Password => &e.password,
        FieldRef::Url { index } => e.urls.get(index).ok_or(AppError::NotFound)?,
        FieldRef::Field { index } => &e.fields.get(index).ok_or(AppError::NotFound)?.value,
        FieldRef::History { index, at } => &history_item(e, index, at)?.password,
    };
    if value.is_empty() {
        return Err(AppError::InvalidInput { field: "field" });
    }
    Ok(Zeroizing::new(value.clone()))
}

/// Value to reveal: only the password and secret custom fields (everything else is
/// already in the entry details).
pub fn secret_value(payload: &Payload, id: Uuid, field: FieldRef) -> AppResult<Zeroizing<String>> {
    let e = find(payload, id)?;
    match field {
        FieldRef::Password => Ok(Zeroizing::new(e.password.clone())),
        FieldRef::Field { index } => {
            let f = e.fields.get(index).ok_or(AppError::NotFound)?;
            if f.secret {
                Ok(Zeroizing::new(f.value.clone()))
            } else {
                Err(AppError::InvalidInput { field: "field" })
            }
        }
        FieldRef::History { index, at } => Ok(Zeroizing::new(history_item(e, index, at)?.password.clone())),
        FieldRef::Username | FieldRef::Url { .. } => Err(AppError::InvalidInput { field: "field" }),
    }
}

fn history_item(e: &Entry, index: usize, at: i64) -> AppResult<&HistoryItem> {
    e.history.get(index).filter(|h| h.at == at).ok_or(AppError::NotFound)
}

/// The entry's password history, newest first, without passwords (I11).
pub fn history(payload: &Payload, id: Uuid) -> AppResult<Vec<HistoryView>> {
    let e = find(payload, id)?;
    Ok(e.history
        .iter()
        .enumerate()
        .rev()
        .map(|(index, h)| HistoryView {
            index,
            at: h.at,
            username: h.username.clone().filter(|u| *u != e.username),
            note: h.note.clone(),
            has_password: !h.password.is_empty(),
        })
        .collect())
}

/// "Use this password": an ordinary password change to the history item's password
/// (the current credentials move into the history), stamped with `ts`.
pub fn use_history_password(payload: &mut Payload, id: Uuid, index: usize, at: i64, ts: i64) -> AppResult<()> {
    let e = find_mut(payload, id)?;
    let password = Zeroizing::new(history_item(e, index, at)?.password.clone());
    if password.is_empty() {
        return Err(AppError::InvalidInput { field: "password" });
    }
    if *password != e.password {
        if !e.password.is_empty() {
            let mut old = HistoryItem::credentials_of(e, None);
            old.at = ts;
            e.push_history(old);
        }
        e.password.clone_from(&password);
    }
    e.updated_at = ts;
    e.normalize_history();
    Ok(())
}

pub fn list(payload: &Payload) -> Vec<EntrySummary> {
    let mut out: Vec<EntrySummary> = payload
        .entries
        .iter()
        .filter(|e| !e.deleted)
        .map(|e| EntrySummary {
            id: e.id,
            title: e.title.clone(),
            username: e.username.clone(),
            url_host: e
                .urls
                .iter()
                .find_map(|u| normalize_url(u))
                .and_then(|u| u.host_str().map(str::to_owned)),
            tags: e.tags.clone(),
            favorite: e.favorite,
            updated_at: e.updated_at,
        })
        .collect();
    out.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

pub fn detail(payload: &Payload, id: Uuid) -> AppResult<EntryDetail> {
    let e = find(payload, id)?;
    Ok(EntryDetail {
        id: e.id,
        title: e.title.clone(),
        username: e.username.clone(),
        has_password: !e.password.is_empty(),
        urls: e.urls.clone(),
        notes: e.notes.clone(),
        tags: e.tags.clone(),
        favorite: e.favorite,
        fields: e
            .fields
            .iter()
            .map(|f| FieldView {
                name: f.name.clone(),
                value: (!f.secret).then(|| f.value.clone()),
                secret: f.secret,
            })
            .collect(),
        history_count: e.history.len(),
        created_at: e.created_at,
        updated_at: e.updated_at,
    })
}

/// Creates or updates an entry stamped with `ts`. A changed password moves the old
/// credentials into the entry's history. Returns the entry id.
pub fn save(payload: &mut Payload, input: &EntryInput, ts: i64) -> AppResult<Uuid> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::InvalidInput { field: "title" });
    }
    // Checked before anything changes; `keep` needs the stored fields.
    let stored: &[CustomField] = match input.id {
        Some(id) => {
            &payload
                .entries
                .iter()
                .find(|e| e.id == id && !e.deleted)
                .ok_or(AppError::NotFound)?
                .fields
        }
        None => &[],
    };
    let fields = input.fields.as_deref().map(|f| custom_fields(f, stored)).transpose()?;
    let entry = match input.id {
        None => {
            payload.entries.push(Entry::new(title, ts));
            payload.entries.last_mut().expect("just pushed")
        }
        Some(id) => find_mut(payload, id)?,
    };

    if let Some(new_password) = &input.password {
        if *new_password != entry.password && !entry.password.is_empty() {
            let mut old = HistoryItem::credentials_of(entry, None);
            old.at = ts;
            entry.push_history(old);
        }
        entry.password.clone_from(new_password);
    }
    entry.title = title.to_owned();
    entry.username = input.username.trim().to_owned();
    entry.urls = clean_list(&input.urls, false);
    entry.notes.clone_from(&input.notes);
    entry.tags = clean_list(&input.tags, true);
    entry.favorite = input.favorite;
    if let Some(fields) = fields {
        entry.fields = fields;
    }
    entry.updated_at = ts;
    entry.normalize_history();
    Ok(entry.id)
}

/// The new field list from the form (V6): names trimmed and non-empty, sizes
/// within the limits, kept values taken from `stored`.
fn custom_fields(input: &[FieldInput], stored: &[CustomField]) -> AppResult<Vec<CustomField>> {
    if input.len() > MAX_FIELDS {
        return Err(AppError::FieldLimit {
            max_fields: MAX_FIELDS,
            max_name: MAX_FIELD_NAME,
            max_value: MAX_FIELD_VALUE,
        });
    }
    input
        .iter()
        .map(|f| {
            let name = f.name.trim();
            if name.is_empty() {
                return Err(AppError::InvalidInput { field: "fieldName" });
            }
            let value = match &f.value {
                FieldValue::Set(value) => value.clone(),
                FieldValue::Keep { index, name } => stored
                    .get(*index)
                    .filter(|s| s.name == *name)
                    .ok_or(AppError::NotFound)?
                    .value
                    .clone(),
            };
            if name.chars().count() > MAX_FIELD_NAME || value.chars().count() > MAX_FIELD_VALUE {
                return Err(AppError::FieldLimit {
                    max_fields: MAX_FIELDS,
                    max_name: MAX_FIELD_NAME,
                    max_value: MAX_FIELD_VALUE,
                });
            }
            Ok(CustomField {
                name: name.to_owned(),
                value,
                secret: f.secret,
            })
        })
        .collect()
}

pub fn delete(payload: &mut Payload, id: Uuid, ts: i64) -> AppResult<()> {
    find_mut(payload, id)?.delete(ts);
    Ok(())
}

pub fn set_favorite(payload: &mut Payload, id: Uuid, favorite: bool, ts: i64) -> AppResult<()> {
    let e = find_mut(payload, id)?;
    e.favorite = favorite;
    e.updated_at = ts;
    Ok(())
}

/// The entry's URL at `index`, normalized, if it is http(s).
pub fn url_to_open(payload: &Payload, id: Uuid, index: usize) -> AppResult<Url> {
    let raw = find(payload, id)?.urls.get(index).ok_or(AppError::NotFound)?;
    normalize_url(raw).ok_or(AppError::InvalidInput { field: "url" })
}

/// Parses a user-entered URL; a missing scheme means `https://`. Only http(s) URLs
/// with a host are accepted.
pub fn normalize_url(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return None;
    }
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then_some(url)
}

fn find(payload: &Payload, id: Uuid) -> AppResult<&Entry> {
    payload
        .entries
        .iter()
        .find(|e| e.id == id && !e.deleted)
        .ok_or(AppError::NotFound)
}

fn find_mut(payload: &mut Payload, id: Uuid) -> AppResult<&mut Entry> {
    payload
        .entries
        .iter_mut()
        .find(|e| e.id == id && !e.deleted)
        .ok_or(AppError::NotFound)
}

/// Trims, drops empty items and (optionally) duplicates, keeping the first order.
fn clean_list(items: &[String], dedupe: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !dedupe || !out.iter().any(|o| o.eq_ignore_ascii_case(item)) {
            out.push(item.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use xpass_core::model::HISTORY_NOTE_IMPORT;

    use super::*;

    fn input(title: &str, password: Option<&str>) -> EntryInput {
        EntryInput {
            id: None,
            title: title.into(),
            username: " alice ".into(),
            password: password.map(Into::into),
            urls: vec![" github.com/login ".into(), "".into()],
            notes: "note".into(),
            tags: vec!["Work".into(), "work".into(), " 2fa ".into()],
            favorite: false,
            fields: None,
        }
    }

    fn set(name: &str, value: &str, secret: bool) -> FieldInput {
        FieldInput {
            name: name.into(),
            secret,
            value: FieldValue::Set(value.into()),
        }
    }

    fn keep(name: &str, index: usize, stored_name: &str, secret: bool) -> FieldInput {
        FieldInput {
            name: name.into(),
            secret,
            value: FieldValue::Keep {
                index,
                name: stored_name.into(),
            },
        }
    }

    /// An entry with a secret `PIN` (1234) and a plain `Branch` (Main).
    fn with_fields() -> (Payload, Uuid) {
        let mut p = Payload::default();
        let mut create = input("Bank", Some("pw"));
        create.fields = Some(vec![set("PIN", "1234", true), set(" Branch ", "Main", false)]);
        let id = save(&mut p, &create, 1).unwrap();
        (p, id)
    }

    fn edit_fields(p: &mut Payload, id: Uuid, fields: Vec<FieldInput>) -> AppResult<Uuid> {
        let mut edit = input("Bank", None);
        edit.id = Some(id);
        edit.fields = Some(fields);
        save(p, &edit, 5)
    }

    fn names(p: &Payload) -> Vec<(String, String, bool)> {
        p.entries[0]
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.value.clone(), f.secret))
            .collect()
    }

    #[test]
    fn fields_are_created_with_trimmed_names() {
        let (p, _) = with_fields();
        assert_eq!(
            names(&p),
            [
                ("PIN".to_owned(), "1234".to_owned(), true),
                ("Branch".to_owned(), "Main".to_owned(), false)
            ]
        );
    }

    #[test]
    fn keep_reorders_renames_and_flips_without_the_value() {
        let (mut p, id) = with_fields();
        edit_fields(
            &mut p,
            id,
            vec![
                keep("Branch office", 1, "Branch", true),
                keep("Card PIN", 0, "PIN", true),
                set("Customer no.", "42", false),
            ],
        )
        .unwrap();
        assert_eq!(
            names(&p),
            [
                ("Branch office".to_owned(), "Main".to_owned(), true),
                ("Card PIN".to_owned(), "1234".to_owned(), true),
                ("Customer no.".to_owned(), "42".to_owned(), false)
            ]
        );
        assert_eq!(p.entries[0].updated_at, 5);
    }

    #[test]
    fn set_replaces_and_a_missing_field_is_deleted() {
        let (mut p, id) = with_fields();
        edit_fields(&mut p, id, vec![set("PIN", "9999", true)]).unwrap();
        assert_eq!(names(&p), [("PIN".to_owned(), "9999".to_owned(), true)]);
        edit_fields(&mut p, id, Vec::new()).unwrap();
        assert!(p.entries[0].fields.is_empty());
    }

    #[test]
    fn none_keeps_the_stored_fields() {
        let (mut p, id) = with_fields();
        let mut edit = input("Bank renamed", None);
        edit.id = Some(id);
        save(&mut p, &edit, 5).unwrap();
        assert_eq!(p.entries[0].fields.len(), 2);
        assert_eq!(p.entries[0].fields[0].value, "1234");
    }

    #[test]
    fn a_stale_keep_changes_nothing() {
        let (mut p, id) = with_fields();
        let before = p.entries[0].clone();
        for stale in [keep("PIN", 0, "Branch", true), keep("PIN", 7, "PIN", true)] {
            assert_eq!(edit_fields(&mut p, id, vec![stale]).unwrap_err(), AppError::NotFound);
        }
        assert_eq!(p.entries[0], before);
        // A new entry has nothing to keep.
        let mut create = input("New", None);
        create.fields = Some(vec![keep("PIN", 0, "PIN", true)]);
        assert_eq!(save(&mut p, &create, 6).unwrap_err(), AppError::NotFound);
        assert_eq!(p.entries.len(), 1, "nothing created");
    }

    #[test]
    fn field_limits() {
        let (mut p, id) = with_fields();
        let limit = AppError::FieldLimit {
            max_fields: MAX_FIELDS,
            max_name: MAX_FIELD_NAME,
            max_value: MAX_FIELD_VALUE,
        };
        let many = (0..=MAX_FIELDS).map(|i| set(&format!("f{i}"), "", false)).collect();
        assert_eq!(edit_fields(&mut p, id, many).unwrap_err(), limit);
        let long_name = "n".repeat(MAX_FIELD_NAME + 1);
        assert_eq!(
            edit_fields(&mut p, id, vec![set(&long_name, "", false)]).unwrap_err(),
            limit
        );
        let long_value = "é".repeat(MAX_FIELD_VALUE + 1);
        assert_eq!(
            edit_fields(&mut p, id, vec![set("Note", &long_value, false)]).unwrap_err(),
            limit
        );
        assert_eq!(
            edit_fields(&mut p, id, vec![set("  ", "x", false)]).unwrap_err(),
            AppError::InvalidInput { field: "fieldName" }
        );
        // At the limits, and duplicate names (imports have them), are fine.
        let full = (0..MAX_FIELDS)
            .map(|_| set("Same", &"v".repeat(MAX_FIELD_VALUE), false))
            .collect();
        edit_fields(&mut p, id, full).unwrap();
        assert_eq!(p.entries[0].fields.len(), MAX_FIELDS);
    }

    #[test]
    fn field_input_wire_format_and_debug() {
        let json = r#"{"id":null,"title":"T","fields":[
            {"name":"PIN","secret":true,"value":{"set":"1234"}},
            {"name":"Old","value":{"keep":{"index":2,"name":"Old"}}}]}"#;
        let parsed: EntryInput = serde_json::from_str(json).unwrap();
        let fields = parsed.fields.as_ref().unwrap();
        assert!(matches!(&fields[0].value, FieldValue::Set(v) if v == "1234"));
        assert!(fields[0].secret && !fields[1].secret);
        assert!(matches!(&fields[1].value, FieldValue::Keep { index: 2, name } if name == "Old"));
        let text = format!("{parsed:?}");
        assert!(!text.contains("1234") && !text.contains("PIN"), "{text}");
        let without: EntryInput = serde_json::from_str(r#"{"id":null,"title":"T"}"#).unwrap();
        assert!(without.fields.is_none());
    }

    #[test]
    fn create_cleans_input_and_stamps_time() {
        let mut p = Payload::default();
        let id = save(&mut p, &input("  GitHub ", Some("pw1")), 100).unwrap();
        let e = &p.entries[0];
        assert_eq!(e.id, id);
        assert_eq!(
            (e.title.as_str(), e.username.as_str(), e.password.as_str()),
            ("GitHub", "alice", "pw1")
        );
        assert_eq!(e.urls, vec!["github.com/login"]);
        assert_eq!(e.tags, vec!["Work", "2fa"]);
        assert_eq!((e.created_at, e.updated_at), (100, 100));
        assert!(e.history.is_empty());
    }

    #[test]
    fn title_is_required() {
        let mut p = Payload::default();
        assert_eq!(
            save(&mut p, &input("  ", None), 1).unwrap_err(),
            AppError::InvalidInput { field: "title" }
        );
        assert!(p.entries.is_empty());
    }

    #[test]
    fn update_keeps_password_when_none_and_records_history_on_change() {
        let mut p = Payload::default();
        let id = save(&mut p, &input("GitHub", Some("pw1")), 100).unwrap();

        let mut edit = input("GitHub", None);
        edit.id = Some(id);
        save(&mut p, &edit, 200).unwrap();
        assert_eq!(p.entries[0].password, "pw1", "None keeps the password");
        assert!(p.entries[0].history.is_empty());

        edit.password = Some("pw2".into());
        save(&mut p, &edit, 300).unwrap();
        let e = &p.entries[0];
        assert_eq!(e.password, "pw2");
        assert_eq!(e.history.len(), 1);
        assert_eq!(
            (
                e.history[0].at,
                e.history[0].password.as_str(),
                e.history[0].username.as_deref()
            ),
            (300, "pw1", Some("alice"))
        );
        assert_eq!(e.updated_at, 300);

        // Same password again: no new history item.
        save(&mut p, &edit, 400).unwrap();
        assert_eq!(p.entries[0].history.len(), 1);
    }

    #[test]
    fn custom_fields_are_kept_and_secret_values_hidden() {
        let mut p = Payload::default();
        let id = save(&mut p, &input("Bank", Some("pw")), 1).unwrap();
        p.entries[0].fields = vec![
            CustomField {
                name: "PIN".into(),
                value: "1234".into(),
                secret: true,
            },
            CustomField {
                name: "Branch".into(),
                value: "Main".into(),
                secret: false,
            },
        ];
        let mut edit = input("Bank", None);
        edit.id = Some(id);
        save(&mut p, &edit, 2).unwrap();
        assert_eq!(p.entries[0].fields.len(), 2);

        let d = detail(&p, id).unwrap();
        assert!(d.has_password);
        assert_eq!(d.fields[0].value, None);
        assert_eq!(d.fields[1].value.as_deref(), Some("Main"));
        let json = serde_json::to_string(&d).unwrap();
        assert!(
            !json.contains("1234") && !json.contains("\"pw\""),
            "no secrets in detail: {json}"
        );
    }

    #[test]
    fn list_is_metadata_only_sorted_and_skips_tombstones() {
        let mut p = Payload::default();
        let a = save(&mut p, &input("zeta", Some("secret-z")), 1).unwrap();
        save(&mut p, &input("Alpha", Some("secret-a")), 2).unwrap();
        let gone = save(&mut p, &input("gone", Some("secret-g")), 3).unwrap();
        delete(&mut p, gone, 4).unwrap();
        set_favorite(&mut p, a, true, 5).unwrap();

        let rows = list(&p);
        assert_eq!(
            rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "zeta"]
        );
        assert_eq!(rows[1].url_host.as_deref(), Some("github.com"));
        assert!(rows[1].favorite && rows[1].updated_at == 5);
        let json = serde_json::to_string(&rows).unwrap();
        assert!(!json.contains("secret-"), "no passwords in list: {json}");
    }

    #[test]
    fn deleted_entries_are_not_found() {
        let mut p = Payload::default();
        let id = save(&mut p, &input("x", None), 1).unwrap();
        delete(&mut p, id, 2).unwrap();
        assert!(p.entries[0].deleted);
        assert_eq!(detail(&p, id).unwrap_err(), AppError::NotFound);
        assert_eq!(delete(&mut p, id, 3).unwrap_err(), AppError::NotFound);
        assert_eq!(
            set_favorite(&mut p, Uuid::nil(), true, 3).unwrap_err(),
            AppError::NotFound
        );
    }

    #[test]
    fn only_http_urls_can_be_opened() {
        assert_eq!(normalize_url("github.com").unwrap().as_str(), "https://github.com/");
        assert_eq!(
            normalize_url("http://intranet:8080/x").unwrap().host_str(),
            Some("intranet")
        );
        for bad in [
            "",
            "   ",
            "javascript:alert(1)",
            "file:///C:/Windows",
            "ms-settings://x",
            "https://",
            "a\nb",
        ] {
            assert!(normalize_url(bad).is_none(), "{bad:?} must be rejected");
        }
        let mut p = Payload::default();
        let mut i = input("x", None);
        i.urls = vec!["file:///C:/x".into(), "example.com".into()];
        let id = save(&mut p, &i, 1).unwrap();
        assert_eq!(
            url_to_open(&p, id, 0).unwrap_err(),
            AppError::InvalidInput { field: "url" }
        );
        assert_eq!(url_to_open(&p, id, 1).unwrap().as_str(), "https://example.com/");
        assert_eq!(url_to_open(&p, id, 2).unwrap_err(), AppError::NotFound);
    }

    #[test]
    fn copy_and_reveal_values() {
        let mut p = Payload::default();
        let id = save(&mut p, &input("Bank", Some("pw")), 1).unwrap();
        p.entries[0].fields = vec![
            CustomField {
                name: "PIN".into(),
                value: "1234".into(),
                secret: true,
            },
            CustomField {
                name: "Branch".into(),
                value: "Main".into(),
                secret: false,
            },
        ];
        let v = |f| field_value(&p, id, f).map(|z| z.to_string());
        assert_eq!(v(FieldRef::Username).unwrap(), "alice");
        assert_eq!(v(FieldRef::Password).unwrap(), "pw");
        assert_eq!(v(FieldRef::Url { index: 0 }).unwrap(), "github.com/login");
        assert_eq!(v(FieldRef::Field { index: 0 }).unwrap(), "1234");
        assert_eq!(v(FieldRef::Url { index: 5 }).unwrap_err(), AppError::NotFound);

        let r = |f| secret_value(&p, id, f).map(|z| z.to_string());
        assert_eq!(r(FieldRef::Password).unwrap(), "pw");
        assert_eq!(r(FieldRef::Field { index: 0 }).unwrap(), "1234");
        assert!(
            r(FieldRef::Field { index: 1 }).is_err(),
            "non-secret field is not revealed"
        );
        assert!(r(FieldRef::Username).is_err());

        let empty = save(&mut p, &input("Empty", None), 2).unwrap();
        assert_eq!(
            field_value(&p, empty, FieldRef::Password).unwrap_err(),
            AppError::InvalidInput { field: "field" }
        );
        let parsed: FieldRef = serde_json::from_str(r#"{"kind":"url","index":1}"#).unwrap();
        assert_eq!(parsed, FieldRef::Url { index: 1 });
    }

    /// Entry with password "pw3" and history pw1 (at 10, other user), pw2 (at 20, import).
    fn with_history() -> (Payload, Uuid) {
        let mut p = Payload::default();
        let id = save(&mut p, &input("Site", Some("pw3")), 30).unwrap();
        let e = &mut p.entries[0];
        e.history = vec![
            HistoryItem {
                at: 10,
                password: "pw1".into(),
                username: Some("old-user".into()),
                note: None,
            },
            HistoryItem {
                at: 20,
                password: "pw2".into(),
                username: Some("alice".into()),
                note: Some(HISTORY_NOTE_IMPORT.into()),
            },
        ];
        (p, id)
    }

    #[test]
    fn history_is_newest_first_without_passwords() {
        let (p, id) = with_history();
        let h = history(&p, id).unwrap();
        assert_eq!(
            h,
            vec![
                HistoryView {
                    index: 1,
                    at: 20,
                    username: None,
                    note: Some("import".into()),
                    has_password: true
                },
                HistoryView {
                    index: 0,
                    at: 10,
                    username: Some("old-user".into()),
                    note: None,
                    has_password: true
                },
            ]
        );
        assert!(!serde_json::to_string(&h).unwrap().contains("pw1"));
    }

    #[test]
    fn history_ref_checks_the_time() {
        let (p, id) = with_history();
        let r = FieldRef::History { index: 0, at: 10 };
        assert_eq!(*secret_value(&p, id, r).unwrap(), "pw1");
        assert_eq!(*field_value(&p, id, r).unwrap(), "pw1");
        let stale = FieldRef::History { index: 0, at: 20 };
        assert_eq!(secret_value(&p, id, stale).unwrap_err(), AppError::NotFound);
        assert_eq!(field_value(&p, id, stale).unwrap_err(), AppError::NotFound);
        let gone = FieldRef::History { index: 5, at: 10 };
        assert_eq!(secret_value(&p, id, gone).unwrap_err(), AppError::NotFound);
    }

    #[test]
    fn history_ref_parses_from_json() {
        let r: FieldRef = serde_json::from_str(r#"{"kind":"history","index":2,"at":99}"#).unwrap();
        assert_eq!(r, FieldRef::History { index: 2, at: 99 });
    }

    #[test]
    fn use_history_password_swaps_and_stamps() {
        let (mut p, id) = with_history();
        use_history_password(&mut p, id, 1, 20, 40).unwrap();
        let e = &p.entries[0];
        assert_eq!(e.password, "pw2");
        assert_eq!(e.updated_at, 40);
        // pw2 + alice is now current, so its history copy is dropped; pw3 moved in.
        let pws: Vec<_> = e.history.iter().map(|h| (h.password.as_str(), h.at)).collect();
        assert_eq!(pws, vec![("pw1", 10), ("pw3", 40)]);

        assert_eq!(
            use_history_password(&mut p, id, 1, 20, 50).unwrap_err(),
            AppError::NotFound,
            "stale ref"
        );
    }

    #[test]
    fn secret_custom_field_is_never_in_details() {
        let (mut p, id) = with_history();
        p.entries[0].fields = vec![CustomField {
            name: "TOTP".into(),
            value: "fake-seed".into(),
            secret: true,
        }];
        let json = serde_json::to_string(&detail(&p, id).unwrap()).unwrap();
        assert!(!json.contains("fake-seed"));
        assert!(!json.contains("pw1"));
    }

    #[test]
    fn entry_input_debug_has_no_values() {
        let input: EntryInput = serde_json::from_str(
            r#"{"title":"Bank","username":"me@example.com","password":"Sample-Pass-01","notes":"pin 1234"}"#,
        )
        .unwrap();
        let text = format!("{input:?}");
        for value in ["Bank", "me@example.com", "Sample-Pass-01", "1234"] {
            assert!(!text.contains(value), "{text}");
        }
    }
}
