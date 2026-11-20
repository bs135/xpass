//! Table + preset (+ generic mapping) → rows ready to import.

use std::fmt;

use serde::Serialize;
use xpass_core::model::CustomField;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    decode::{Record, Table},
    error::ImportError,
    preset::{find, guess_mapping, Mapping, Preset, Unmapped},
    text::{clean_list, host_of, is_truthy},
    time::parse_iso8601,
};

/// Custom field name for one-time password secrets (`otpauth://` URIs or raw secrets).
pub const TOTP_FIELD: &str = "TOTP";

/// One entry to import. Zeroized on drop; `Debug` shows no secrets.
#[derive(Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct ImportRow {
    /// Line in the file where the row starts.
    pub line: u64,
    pub title: String,
    pub username: String,
    pub password: String,
    pub urls: Vec<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub fields: Vec<CustomField>,
    /// Creation date from the source (KeePassXC, xPass), Unix ms.
    pub created_at: Option<i64>,
    #[zeroize(skip)]
    pub warnings: Vec<RowWarning>,
}

impl fmt::Debug for ImportRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportRow")
            .field("line", &self.line)
            .field("title", &self.title)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

/// Something the user may want to know about a row; it is still imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RowWarning {
    /// No title in the file; the URL host is used.
    TitleFromUrl,
    /// No title and no URL; the username is used.
    TitleFromUsername,
    /// The creation date could not be read; the import time is used.
    InvalidDate,
    /// The xPass `fields` cell could not be read; custom fields were dropped.
    InvalidFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// No title, username, password, URL or notes.
    Empty,
    /// A Bitwarden item type other than login or secure note (card, identity, …).
    UnsupportedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRow {
    pub line: u64,
    pub reason: SkipReason,
}

/// Result of [`convert`].
#[derive(Debug)]
pub struct Parsed {
    pub preset: Preset,
    /// The generic mapping used (given or guessed); `None` for the other presets.
    pub mapping: Option<Mapping>,
    pub rows: Vec<ImportRow>,
    pub skipped: Vec<SkippedRow>,
}

/// Converts every data row with `preset`. `mapping` is only used for
/// [`Preset::Generic`]; `None` guesses it from the headers.
pub fn convert(table: &Table, preset: Preset, mapping: Option<&Mapping>) -> Result<Parsed, ImportError> {
    let columns = Columns::new(&table.headers, preset, mapping)?;
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for record in &table.rows {
        match columns.row(record, &table.headers) {
            Ok(row) => rows.push(row),
            Err(reason) => skipped.push(SkippedRow {
                line: record.line,
                reason,
            }),
        }
    }
    Ok(Parsed {
        preset,
        mapping: (preset == Preset::Generic).then(|| columns.mapping.clone()),
        rows,
        skipped,
    })
}

/// Resolved column indices for one preset.
struct Columns {
    preset: Preset,
    mapping: Mapping,
    totp: Option<usize>,
    created: Option<usize>,
    fields: Option<usize>,
    kind: Option<usize>,
    /// Bitwarden organization exports have `collections` (comma-separated) instead of `folder`.
    collections: bool,
}

impl Columns {
    fn new(headers: &[String], preset: Preset, mapping: Option<&Mapping>) -> Result<Self, ImportError> {
        let f = |names: &[&str]| find(headers, names);
        let mut columns = Columns {
            preset,
            mapping: Mapping::default(),
            totp: None,
            created: None,
            fields: None,
            kind: None,
            collections: false,
        };
        columns.mapping = match preset {
            Preset::Bitwarden => {
                columns.totp = f(&["logintotp"]);
                columns.fields = f(&["fields"]);
                columns.kind = f(&["type"]);
                let folder = f(&["folder"]);
                columns.collections = folder.is_none();
                Mapping {
                    title: f(&["name"]),
                    username: f(&["loginusername"]),
                    password: f(&["loginpassword"]),
                    url: f(&["loginuri"]),
                    notes: f(&["notes"]),
                    tags: folder.or_else(|| f(&["collections"])),
                    favorite: f(&["favorite"]),
                    unmapped: Unmapped::Ignore,
                }
            }
            Preset::Chrome => Mapping {
                title: f(&["name"]),
                username: f(&["username"]),
                password: f(&["password"]),
                url: f(&["url"]),
                notes: f(&["note", "notes"]),
                ..Mapping::default()
            },
            Preset::KeePassXc => {
                columns.totp = f(&["totp"]);
                columns.created = f(&["created"]);
                Mapping {
                    title: f(&["title"]),
                    username: f(&["username"]),
                    password: f(&["password"]),
                    url: f(&["url"]),
                    notes: f(&["notes"]),
                    tags: f(&["group"]),
                    ..Mapping::default()
                }
            }
            Preset::XPass => {
                columns.created = f(&["created"]);
                columns.fields = f(&["fields"]);
                Mapping {
                    title: f(&["title"]),
                    username: f(&["username"]),
                    password: f(&["password"]),
                    url: f(&["urls"]),
                    notes: f(&["notes"]),
                    tags: f(&["tags"]),
                    favorite: f(&["favorite"]),
                    unmapped: Unmapped::Ignore,
                }
            }
            Preset::Generic => {
                let mapping = mapping.cloned().unwrap_or_else(|| guess_mapping(headers));
                mapping.validate(headers.len())?;
                mapping
            }
        };
        Ok(columns)
    }

    fn row(&self, r: &Record, headers: &[String]) -> Result<ImportRow, SkipReason> {
        let m = &self.mapping;
        if let Some(kind) = self.kind {
            if !matches!(
                r.cell(Some(kind)).trim().to_ascii_lowercase().as_str(),
                "" | "login" | "note"
            ) {
                return Err(SkipReason::UnsupportedType);
            }
        }

        // `ImportRow` zeroizes on drop, so struct-update syntax is not available.
        let mut row = ImportRow::default();
        row.line = r.line;
        row.title = r.cell(m.title).trim().to_owned();
        row.username = r.cell(m.username).trim().to_owned();
        // Passwords are kept exactly as exported (spaces may be intended).
        row.password = r.cell(m.password).to_owned();
        row.urls = self.urls(r.cell(m.url));
        row.notes = r.cell(m.notes).trim_end().to_owned();
        row.tags = self.tags(r.cell(m.tags));
        row.favorite = is_truthy(r.cell(m.favorite));
        if row.title.is_empty()
            && row.username.is_empty()
            && row.password.is_empty()
            && row.urls.is_empty()
            && row.notes.trim().is_empty()
        {
            return Err(SkipReason::Empty);
        }

        if let Some(fields) = self.fields {
            self.read_fields(r.cell(Some(fields)), &mut row);
        }
        let totp = r.cell(self.totp).trim();
        if !totp.is_empty() {
            row.fields.push(CustomField {
                name: TOTP_FIELD.to_owned(),
                value: totp.to_owned(),
                secret: true,
            });
        }
        if let Some(created) = self.created {
            let raw = r.cell(Some(created));
            if !raw.trim().is_empty() {
                row.created_at = parse_iso8601(raw);
                if row.created_at.is_none() {
                    row.warnings.push(RowWarning::InvalidDate);
                }
            }
        }
        if self.preset == Preset::Generic {
            self.unmapped(r, headers, &mut row);
        }

        if row.title.is_empty() {
            if let Some(host) = row.urls.iter().find_map(|u| host_of(u)) {
                row.title = host;
                row.warnings.push(RowWarning::TitleFromUrl);
            } else if !row.username.is_empty() {
                row.title.clone_from(&row.username);
                row.warnings.push(RowWarning::TitleFromUsername);
            } else {
                // Only notes (or a password): a title is required by the app.
                row.title = "Imported".to_owned();
            }
        }
        Ok(row)
    }

    fn urls(&self, cell: &str) -> Vec<String> {
        match self.preset {
            // Bitwarden joins several URIs with commas.
            Preset::Bitwarden => clean_list(cell.split([',', '\n']), false),
            Preset::Chrome | Preset::KeePassXc => clean_list([cell], false),
            Preset::XPass | Preset::Generic => clean_list(cell.lines(), false),
        }
    }

    fn tags(&self, cell: &str) -> Vec<String> {
        match self.preset {
            Preset::Bitwarden if self.collections => clean_list(cell.split(','), true),
            Preset::Bitwarden => clean_list([cell], true),
            Preset::KeePassXc => {
                // KeePassXC writes the group path from the root group, `Root/Email`.
                let path = cell.trim();
                let path = path
                    .strip_prefix("Root/")
                    .unwrap_or(if path == "Root" { "" } else { path });
                clean_list([path], true)
            }
            Preset::XPass => clean_list(cell.lines(), true),
            Preset::Chrome => Vec::new(),
            Preset::Generic => clean_list(cell.split([',', ';', '\n']), true),
        }
    }

    fn read_fields(&self, cell: &str, row: &mut ImportRow) {
        if cell.trim().is_empty() {
            return;
        }
        if self.preset == Preset::XPass {
            match serde_json::from_str::<Vec<CustomField>>(cell) {
                Ok(fields) => row.fields.extend(fields),
                Err(_) => row.warnings.push(RowWarning::InvalidFields),
            }
            return;
        }
        // Bitwarden: one `name: value` per line.
        for line in cell.lines().filter(|l| !l.trim().is_empty()) {
            let (name, value) = line.split_once(':').unwrap_or((line, ""));
            row.fields.push(CustomField {
                name: name.trim().to_owned(),
                value: value.strip_prefix(' ').unwrap_or(value).to_owned(),
                secret: false,
            });
        }
    }

    fn unmapped(&self, r: &Record, headers: &[String], row: &mut ImportRow) {
        if self.mapping.unmapped == Unmapped::Ignore {
            return;
        }
        let used: Vec<usize> = self.mapping.columns().collect();
        for (i, header) in headers.iter().enumerate() {
            let value = r.cell(Some(i)).trim();
            if used.contains(&i) || value.is_empty() {
                continue;
            }
            let name = if header.is_empty() {
                format!("Column {}", i + 1)
            } else {
                header.clone()
            };
            match self.mapping.unmapped {
                Unmapped::Notes => {
                    if !row.notes.is_empty() {
                        row.notes.push('\n');
                    }
                    row.notes.push_str(&format!("{name}: {value}"));
                }
                Unmapped::Fields => row.fields.push(CustomField {
                    name,
                    value: value.to_owned(),
                    secret: false,
                }),
                Unmapped::Ignore => {}
            }
        }
    }
}
