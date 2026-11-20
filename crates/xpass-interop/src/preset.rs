//! CSV presets (plan I4): detection from the header row and the generic column mapping.

use serde::{Deserialize, Serialize};

use crate::error::ImportError;

/// Where a CSV file comes from; decides how columns are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    #[serde(rename = "bitwarden")]
    Bitwarden,
    /// Chrome, Edge and other Chromium browsers.
    #[serde(rename = "chrome")]
    Chrome,
    #[serde(rename = "keepassxc")]
    KeePassXc,
    /// The layout written by [`crate::csv_export`].
    #[serde(rename = "xpass")]
    XPass,
    /// Any other file, read through a [`Mapping`].
    #[serde(rename = "generic")]
    Generic,
}

/// Column header written by the xPass CSV export (plan I8).
pub const XPASS_HEADERS: [&str; 10] = [
    "title", "username", "password", "urls", "notes", "tags", "favorite", "created", "updated", "fields",
];

/// What to do with columns a generic mapping does not use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Unmapped {
    #[default]
    Ignore,
    /// Appended to the notes as `Header: value` lines.
    Notes,
    /// Stored as (non-secret) custom fields named after the header.
    Fields,
}

/// Column indices for a generic file (`None` = not imported).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub title: Option<usize>,
    pub username: Option<usize>,
    pub password: Option<usize>,
    /// Several URLs in one cell are separated by new lines.
    pub url: Option<usize>,
    pub notes: Option<usize>,
    /// Several tags in one cell are separated by `,`, `;` or new lines.
    pub tags: Option<usize>,
    pub favorite: Option<usize>,
    #[serde(default)]
    pub unmapped: Unmapped,
}

impl Mapping {
    /// Mapped column indices, in field order.
    pub fn columns(&self) -> impl Iterator<Item = usize> {
        [
            self.title,
            self.username,
            self.password,
            self.url,
            self.notes,
            self.tags,
            self.favorite,
        ]
        .into_iter()
        .flatten()
    }

    /// Every index must exist, and at least one of title / username / password / URL
    /// must be mapped (otherwise every row would be empty).
    pub fn validate(&self, column_count: usize) -> Result<(), ImportError> {
        if let Some(index) = self.columns().find(|i| *i >= column_count) {
            return Err(ImportError::NoSuchColumn { index });
        }
        if self.title.is_none() && self.username.is_none() && self.password.is_none() && self.url.is_none() {
            return Err(ImportError::NothingMapped);
        }
        Ok(())
    }
}

/// Header normalized for comparison: lower-case ASCII letters and digits only
/// (`login_uri` → `loginuri`, `Last Modified` → `lastmodified`).
pub(crate) fn norm(header: &str) -> String {
    header
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Index of the first header whose normalized form is one of `names`.
pub(crate) fn find(headers: &[String], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .find_map(|name| headers.iter().position(|h| norm(h) == *name))
}

fn has_all(headers: &[String], names: &[&str]) -> bool {
    names.iter().all(|n| find(headers, &[n]).is_some())
}

/// Recognizes the exporting app from the header row.
pub fn detect(headers: &[String]) -> Preset {
    let normalized: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    if normalized.iter().map(String::as_str).eq(XPASS_HEADERS) {
        Preset::XPass
    } else if has_all(headers, &["type", "name", "loginuri", "loginusername", "loginpassword"]) {
        Preset::Bitwarden
    } else if has_all(headers, &["group", "title", "username", "password", "url", "notes"]) {
        Preset::KeePassXc
    } else if has_all(headers, &["name", "url", "username", "password"]) {
        Preset::Chrome
    } else {
        Preset::Generic
    }
}

/// First guess of a generic mapping from common header names; each column is used once.
pub fn guess_mapping(headers: &[String]) -> Mapping {
    let mut used = Vec::new();
    let mut pick = |names: &[&str]| {
        let found = names.iter().find_map(|name| {
            headers
                .iter()
                .enumerate()
                .position(|(i, h)| !used.contains(&i) && norm(h) == *name)
        });
        if let Some(i) = found {
            used.push(i);
        }
        found
    };
    // Order matters: specific names first, so `website` is a URL and `name` a title.
    let password = pick(&["password", "pass", "passwd", "pwd", "loginpassword"]);
    let username = pick(&[
        "username",
        "user",
        "login",
        "loginname",
        "loginusername",
        "email",
        "userid",
        "account",
    ]);
    let url = pick(&[
        "url",
        "urls",
        "uri",
        "loginuri",
        "website",
        "web",
        "webaddress",
        "address",
        "link",
        "hostname",
    ]);
    let title = pick(&["title", "name", "site", "sitename", "service", "entry", "label"]);
    let notes = pick(&["notes", "note", "comment", "comments", "description", "extra"]);
    let tags = pick(&["tags", "tag", "folder", "group", "category", "grouping", "labels"]);
    let favorite = pick(&["favorite", "favourite", "fav", "starred"]);
    Mapping {
        title,
        username,
        password,
        url,
        notes,
        tags,
        favorite,
        unmapped: Unmapped::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<String> {
        s.split(',').map(str::to_owned).collect()
    }

    #[test]
    fn detects_presets() {
        let cases = [
            (
                "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp",
                Preset::Bitwarden,
            ),
            (
                "collections,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp",
                Preset::Bitwarden,
            ),
            ("name,url,username,password,note", Preset::Chrome),
            ("name,url,username,password", Preset::Chrome),
            (
                "Group,Title,Username,Password,URL,Notes,TOTP,Icon,Last Modified,Created",
                Preset::KeePassXc,
            ),
            (
                "title,username,password,urls,notes,tags,favorite,created,updated,fields",
                Preset::XPass,
            ),
            ("Website,Login,Pass", Preset::Generic),
            ("title,username,password,urls", Preset::Generic),
        ];
        for (headers, preset) in cases {
            assert_eq!(detect(&h(headers)), preset, "{headers}");
        }
    }

    #[test]
    fn guesses_generic_mapping() {
        let m = guess_mapping(&h("Website,Login,Pass,Comment,Category,Extra,Name"));
        assert_eq!(
            m,
            Mapping {
                title: Some(6),
                username: Some(1),
                password: Some(2),
                url: Some(0),
                notes: Some(3),
                tags: Some(4),
                favorite: None,
                unmapped: Unmapped::Ignore,
            }
        );
        // `Extra` was taken by nothing else, so it is the notes column when `Comment` is absent.
        assert_eq!(guess_mapping(&h("user,extra")).notes, Some(1));
    }

    #[test]
    fn validates_mapping() {
        let ok = Mapping {
            password: Some(1),
            ..Mapping::default()
        };
        assert_eq!(ok.validate(2), Ok(()));
        assert_eq!(ok.validate(1), Err(ImportError::NoSuchColumn { index: 1 }));
        let nothing = Mapping {
            notes: Some(0),
            ..Mapping::default()
        };
        assert_eq!(nothing.validate(1), Err(ImportError::NothingMapped));
    }
}
