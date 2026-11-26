//! KeePass (KDBX 4) export (plan I9).
//!
//! One group `xPass` under the root. Per entry: Title, UserName, Password
//! (protected), URL = first URL and more URLs as `KP2A_URL_1…` (KeePassXC
//! convention), Notes, tags (favorite → tag `favorite`), custom fields as string
//! fields (secret → protected), password history as KDBX entry history, creation /
//! modification times. Deleted entries (tombstones) are not exported.
//!
//! File: KDBX 4, ChaCha20 outer and inner cipher, GZip, Argon2id ([`KdbxOptions`]),
//! protected by a separate export password.
//!
//! Known limit: the `keepass` crate keeps field values in `Value<String>` (zeroized
//! when protected) but its XML buffers are not zeroized; dropped right after saving.

use chrono::{DateTime, NaiveDateTime};
use keepass::{
    config::{CompressionConfig, DatabaseConfig, InnerCipherConfig, KdfConfig, OuterCipherConfig},
    db::{fields, Entry as KpEntry, Times, Value},
    Database, DatabaseKey,
};
use xpass_core::{
    model::{Entry, Payload},
    otp,
};
use zeroize::Zeroizing;

/// Name of the group holding every exported entry.
pub const GROUP_NAME: &str = "xPass";
/// Tag that marks favorites.
pub const FAVORITE_TAG: &str = "favorite";
/// KeePassXC / KeePass2Android field name prefix for extra URLs (`KP2A_URL_1`, …).
pub const EXTRA_URL_PREFIX: &str = "KP2A_URL_";
/// KeePassXC's one-time password attribute (an `otpauth://` URI).
pub const OTP_FIELD: &str = "otp";

/// Argon2id parameters of the exported file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdbxOptions {
    pub memory_bytes: u64,
    pub iterations: u64,
    pub parallelism: u32,
}

impl KdbxOptions {
    /// 64 MiB, 2 lanes, 25 passes: about one second to open on a current desktop
    /// CPU (measured 0.4 s for 10 passes in a release build).
    pub const DEFAULT: Self = Self {
        memory_bytes: 64 * 1024 * 1024,
        iterations: 25,
        parallelism: 2,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KdbxError {
    #[error("the export password is empty")]
    EmptyPassword,
    /// Writing the file failed inside the `keepass` crate (no details: they could
    /// carry data).
    #[error("the KeePass file could not be written")]
    Write,
}

/// Builds a KDBX 4 file of the live entries of `payload`, encrypted with
/// `password`. Returns the file bytes and the number of entries.
pub fn export_kdbx(
    payload: &Payload,
    password: &str,
    options: &KdbxOptions,
) -> Result<(Zeroizing<Vec<u8>>, usize), KdbxError> {
    if password.is_empty() {
        return Err(KdbxError::EmptyPassword);
    }
    let mut db = Database::with_config(config(options));
    db.meta.generator = Some("xPass".into());
    db.meta.database_name = Some("xPass export".into());

    let mut count = 0;
    {
        let mut root = db.root_mut();
        root.name = "Root".into();
        let mut group = root.add_group();
        group.name = GROUP_NAME.into();
        for e in payload.entries.iter().filter(|e| !e.deleted) {
            let mut kp = group.add_entry();
            fill(&mut kp, e);
            count += 1;
        }
    }

    let mut out = Zeroizing::new(Vec::new());
    db.save(&mut *out, DatabaseKey::new().with_password(password))
        .map_err(|_| KdbxError::Write)?;
    Ok((out, count))
}

fn config(options: &KdbxOptions) -> DatabaseConfig {
    let mut config = DatabaseConfig::default();
    config.outer_cipher_config = OuterCipherConfig::ChaCha20;
    config.inner_cipher_config = InnerCipherConfig::ChaCha20;
    config.compression_config = CompressionConfig::GZip;
    config.kdf_config = KdfConfig::Argon2id {
        iterations: options.iterations,
        memory: options.memory_bytes,
        parallelism: options.parallelism,
        version: argon2::Version::Version13,
    };
    config
}

/// Sets the current values and the history of `kp` from `e`.
fn fill(kp: &mut KpEntry, e: &Entry) {
    kp.set_unprotected(fields::TITLE, e.title.as_str());
    kp.set_unprotected(fields::USERNAME, e.username.as_str());
    kp.set_protected(fields::PASSWORD, e.password.as_str());
    kp.set_unprotected(fields::URL, e.urls.first().map_or("", String::as_str));
    for (i, url) in e.urls.iter().enumerate().skip(1) {
        kp.set_unprotected(format!("{EXTRA_URL_PREFIX}{i}"), url.as_str());
    }
    kp.set_unprotected(fields::NOTES, e.notes.as_str());
    for f in &e.fields {
        let value = if f.secret {
            Value::protected(f.value.as_str())
        } else {
            Value::unprotected(f.value.as_str())
        };
        kp.set(field_name(kp, &f.name), value);
    }
    // KeePassXC reads one-time passwords from `otp` (v0.8 plan V2); the field stays too.
    let totp = e
        .fields
        .iter()
        .filter(|f| otp::is_totp_field(f))
        .find_map(|f| otp::Totp::parse(&f.value).ok());
    if let (Some(totp), false) = (totp, kp.fields.contains_key(OTP_FIELD)) {
        kp.set_protected(OTP_FIELD, totp.to_uri().as_str());
    }
    kp.tags.clone_from(&e.tags);
    if e.favorite && !kp.tags.iter().any(|t| t == FAVORITE_TAG) {
        kp.tags.push(FAVORITE_TAG.into());
    }
    kp.times = times(e.created_at, e.updated_at);

    // KDBX history: oldest first; each item is the entry as it was, with the old
    // credentials and the time it was replaced. `History::add_entry` inserts at the
    // front, so the newest item goes in first.
    let current = kp.clone();
    let mut history = keepass::db::History::default();
    for item in e.history.iter().rev() {
        let mut old = current.clone();
        old.set_protected(fields::PASSWORD, item.password.as_str());
        if let Some(username) = &item.username {
            old.set_unprotected(fields::USERNAME, username.as_str());
        }
        old.times = times(e.created_at, item.at);
        history.add_entry(old);
    }
    kp.history = Some(history);
}

/// A custom field name that does not overwrite a standard field or a field already
/// set (KDBX field names are unique).
fn field_name(kp: &KpEntry, name: &str) -> String {
    let base = if name.trim().is_empty() { "Field" } else { name };
    let taken = |n: &str| fields::KNOWN_FIELDS.contains(&n) || kp.fields.contains_key(n);
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_owned())
}

fn times(created_ms: i64, modified_ms: i64) -> Times {
    let mut t = Times::default();
    t.creation = Some(naive(created_ms));
    t.last_modification = Some(naive(modified_ms));
    t.last_access = Some(naive(modified_ms));
    t.location_changed = Some(naive(created_ms));
    t.expiry = naive(modified_ms);
    t.expires = Some(false);
    t.usage_count = Some(0);
    t
}

/// Unix milliseconds → UTC time with second precision (all KDBX keeps).
fn naive(ms: i64) -> NaiveDateTime {
    DateTime::from_timestamp(ms.div_euclid(1000), 0)
        .unwrap_or_default()
        .naive_utc()
}

#[cfg(test)]
mod tests {
    use xpass_core::model::{CustomField, HistoryItem, HISTORY_NOTE_IMPORT};

    use super::*;

    /// Cheap KDF so the tests stay fast.
    const FAST: KdbxOptions = KdbxOptions {
        memory_bytes: 1024 * 1024,
        iterations: 1,
        parallelism: 1,
    };
    const EXPORT_PW: &str = "export-test-password";

    fn sample() -> Payload {
        let mut p = Payload::default();
        let mut a = Entry::new("Example", 1_700_000_000_000);
        a.updated_at = 1_700_000_500_000;
        a.username = "alice".into();
        a.password = "fake-pass-new".into();
        a.urls = vec!["https://example.com".into(), "https://login.example.com".into()];
        a.notes = "line 1\nline 2".into();
        a.tags = vec!["work".into()];
        a.favorite = true;
        a.fields = vec![
            CustomField {
                name: "TOTP".into(),
                value: "fake-totp-seed".into(),
                secret: true,
            },
            CustomField {
                name: "Title".into(),
                value: "clash".into(),
                secret: false,
            },
        ];
        a.history = vec![
            HistoryItem {
                at: 1_700_000_100_000,
                password: "fake-pass-oldest".into(),
                username: Some("alice-old".into()),
                note: None,
            },
            HistoryItem {
                at: 1_700_000_200_000,
                password: "fake-pass-old".into(),
                username: None,
                note: Some(HISTORY_NOTE_IMPORT.into()),
            },
        ];
        p.entries.push(a);

        let mut gone = Entry::new("Deleted", 1_700_000_000_000);
        gone.deleted = true;
        p.entries.push(gone);
        p
    }

    #[test]
    fn totp_fields_also_become_keepassxc_otp() {
        let mut p = sample();
        p.entries[0].fields.push(CustomField {
            name: "OTP".into(),
            value: "gezd gnbv gy3t qojq gezd gnbv gy3t qojq".into(),
            secret: true,
        });
        let (bytes, _) = export_kdbx(&p, EXPORT_PW, &FAST).unwrap();
        let db = open(&bytes, EXPORT_PW);
        let root = db.root();
        let group = root.group_by_name(GROUP_NAME).unwrap();
        let e = group.entry_by_name("Example").unwrap();
        assert_eq!(
            e.get(OTP_FIELD),
            Some("otpauth://totp/?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")
        );
        assert!(e.fields[OTP_FIELD].is_protected());
        assert!(e.get("OTP").is_some(), "the field itself stays");
    }

    fn open(bytes: &[u8], pw: &str) -> Database {
        Database::parse(bytes, DatabaseKey::new().with_password(pw)).unwrap()
    }

    #[test]
    fn round_trip_with_the_keepass_crate() {
        let (bytes, count) = export_kdbx(&sample(), EXPORT_PW, &FAST).unwrap();
        assert_eq!(count, 1, "tombstones are not exported");
        let db = open(&bytes, EXPORT_PW);
        assert_eq!(db.num_entries(), 1);
        let root = db.root();
        let group = root.group_by_name(GROUP_NAME).expect("xPass group");
        let e = group.entry_by_name("Example").expect("entry");

        assert_eq!(e.get_username(), Some("alice"));
        assert_eq!(e.get_password(), Some("fake-pass-new"));
        assert!(e.fields[fields::PASSWORD].is_protected());
        assert_eq!(e.get_url(), Some("https://example.com"));
        assert_eq!(e.get("KP2A_URL_1"), Some("https://login.example.com"));
        assert_eq!(e.get(fields::NOTES), Some("line 1\nline 2"));
        assert_eq!(e.tags, vec!["work".to_owned(), FAVORITE_TAG.to_owned()]);
        assert_eq!(e.get("TOTP"), Some("fake-totp-seed"));
        assert!(e.fields["TOTP"].is_protected());
        assert_eq!(e.get(OTP_FIELD), None, "not a usable secret: no otp attribute");
        assert_eq!(
            e.get("Title (2)"),
            Some("clash"),
            "custom field does not overwrite Title"
        );
        assert!(!e.fields["Title (2)"].is_protected());
        assert_eq!(e.times.creation, Some(naive(1_700_000_000_000)));
        assert_eq!(e.times.last_modification, Some(naive(1_700_000_500_000)));

        let history = e.history.as_ref().expect("history").get_entries();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].get_password(), Some("fake-pass-oldest"), "oldest first");
        assert_eq!(history[0].get_username(), Some("alice-old"));
        assert_eq!(history[0].times.last_modification, Some(naive(1_700_000_100_000)));
        assert_eq!(history[1].get_password(), Some("fake-pass-old"));
        assert_eq!(history[1].get_username(), Some("alice"));
    }

    #[test]
    fn file_settings() {
        let (bytes, _) = export_kdbx(&sample(), EXPORT_PW, &FAST).unwrap();
        let db = open(&bytes, EXPORT_PW);
        assert_eq!(db.config.outer_cipher_config, OuterCipherConfig::ChaCha20);
        assert!(matches!(
            db.config.kdf_config,
            KdfConfig::Argon2id {
                iterations: 1,
                memory: 1_048_576,
                ..
            }
        ));
        assert!(matches!(db.config.version, keepass::config::DatabaseVersion::KDB4(_)));
        assert!(!bytes.windows(13).any(|w| w == b"fake-pass-new"), "encrypted");
    }

    #[test]
    fn wrong_password_does_not_open() {
        let (bytes, _) = export_kdbx(&sample(), EXPORT_PW, &FAST).unwrap();
        assert!(Database::parse(&bytes, DatabaseKey::new().with_password("nope")).is_err());
    }

    #[test]
    fn empty_password_is_refused() {
        assert_eq!(export_kdbx(&sample(), "", &FAST).unwrap_err(), KdbxError::EmptyPassword);
    }

    #[test]
    fn empty_vault_exports_an_empty_group() {
        let (bytes, count) = export_kdbx(&Payload::default(), EXPORT_PW, &FAST).unwrap();
        assert_eq!(count, 0);
        let db = open(&bytes, EXPORT_PW);
        assert!(db.root().group_by_name(GROUP_NAME).is_some());
    }
}
