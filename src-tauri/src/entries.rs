//! Entry DTOs for the frontend and the payload operations behind the entry commands.
//!
//! Contract (PLAN.md §3.5): `list` returns metadata only, `detail` never contains the
//! password or secret custom field values, and `save` takes `password: None` to keep
//! the current one. Secrets reach the WebView only through explicit reveal (T7).

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;
use xpass_core::model::{Entry, HistoryItem, Payload};
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

/// Create (`id: None`) or update an entry. Custom fields are kept as they are
/// (no UI for them in v0.1).
#[derive(Debug, Deserialize, Zeroize, ZeroizeOnDrop)]
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
}

/// A copyable/revealable part of an entry.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldRef {
    Username,
    Password,
    Url { index: usize },
    Field { index: usize },
}

/// Value to copy. Empty values are rejected so a copy never just clears the clipboard.
pub fn field_value(payload: &Payload, id: Uuid, field: FieldRef) -> AppResult<Zeroizing<String>> {
    let e = find(payload, id)?;
    let value = match field {
        FieldRef::Username => &e.username,
        FieldRef::Password => &e.password,
        FieldRef::Url { index } => e.urls.get(index).ok_or(AppError::NotFound)?,
        FieldRef::Field { index } => &e.fields.get(index).ok_or(AppError::NotFound)?.value,
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
        FieldRef::Username | FieldRef::Url { .. } => Err(AppError::InvalidInput { field: "field" }),
    }
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
    entry.updated_at = ts;
    entry.normalize_history();
    Ok(entry.id)
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
    use xpass_core::model::CustomField;

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
        }
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
}
