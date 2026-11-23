//! Vault export (v0.5 plan I7–I10): plaintext CSV or a KeePass (KDBX 4) file.
//!
//! Order of the command: the UI confirms → [`AppState::reauthorize`] → save dialog
//! → [`AppState::export_to`]. The file is built from a snapshot of the payload
//! taken after the dialog and written next to the target (temp file + rename, never
//! `%TEMP%`, no second copy). If the vault is locked before the write, nothing is
//! written.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use xpass_interop::{
    csv_export::export_csv,
    kdbx_export::{export_kdbx, KdbxError, KdbxOptions},
    time::format_iso8601,
};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Csv,
    Kdbx,
}

/// Argon2id cost of KDBX exports (cheap in unit tests, which only check the format).
#[cfg(not(test))]
const KDBX_OPTIONS: KdbxOptions = KdbxOptions::DEFAULT;
#[cfg(test)]
const KDBX_OPTIONS: KdbxOptions = KdbxOptions {
    memory_bytes: 1024 * 1024,
    iterations: 1,
    parallelism: 1,
};

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Kdbx => "kdbx",
        }
    }
}

/// What the UI shows after an export: no folder, only the file name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub file_name: String,
    pub count: usize,
}

/// `xpass-export-YYYY-MM-DD.<ext>` (UTC date), proposed in the save dialog.
pub fn suggested_file_name(format: ExportFormat, now_ms: i64) -> String {
    let date = format_iso8601(now_ms);
    format!("xpass-export-{}.{}", &date[..10], format.extension())
}

impl AppState {
    /// Checks the KDBX export password before anything else happens (so the user
    /// is not asked to authenticate for an export that would be refused).
    pub fn check_export_password(format: ExportFormat, kdbx_password: Option<&str>) -> AppResult<()> {
        if format == ExportFormat::Kdbx {
            let password = kdbx_password.ok_or(AppError::InvalidInput { field: "kdbxPassword" })?;
            if !strength::is_strong_enough(password) {
                return Err(AppError::WeakPassword);
            }
        }
        Ok(())
    }

    /// Writes the live entries to `path` in `format`. The caller has already
    /// re-authenticated the user and shown the save dialog.
    pub fn export_to(&self, path: &Path, format: ExportFormat, kdbx_password: Option<&str>) -> AppResult<ExportResult> {
        Self::check_export_password(format, kdbx_password)?;
        // Snapshot under the session lock; the KDF runs without holding it.
        let payload = self.read(|p| Ok(p.clone()))?;
        let (bytes, count) = match format {
            ExportFormat::Csv => export_csv(&payload),
            ExportFormat::Kdbx => {
                let password = kdbx_password.ok_or(AppError::InvalidInput { field: "kdbxPassword" })?;
                export_kdbx(&payload, password, &KDBX_OPTIONS).map_err(|e| match e {
                    KdbxError::EmptyPassword => AppError::WeakPassword,
                    KdbxError::Write => AppError::Internal,
                })?
            }
        };
        drop(payload);
        // Locked while the file was being built: write nothing.
        if self.status() != VaultStatus::Unlocked {
            return Err(AppError::Locked);
        }
        write_replacing(path, &bytes)?;
        Ok(ExportResult {
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            count,
        })
    }
}

/// Writes `bytes` to a temp file in the target folder, then renames it over `path`
/// (an existing file is replaced; the save dialog already asked). The temp file is
/// removed on failure.
fn write_replacing(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path.parent().ok_or(AppError::InvalidInput { field: "path" })?;
    let tmp = temp_path(dir);
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(AppError::from)
}

fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(".xpass-export-{}.tmp", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use keepass::{Database, DatabaseKey};
    use xpass_core::model::Entry;
    use xpass_interop::{decode::decode, preset::XPASS_HEADERS};

    use super::*;
    use crate::{hello::fake::FakeHello, paths::Paths};

    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";
    const EXPORT_PW: &str = "tidy otter lantern 42 meadow";

    fn unlocked() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let clipboard = SecureClipboard::new(crate::clipboard::fake::FakeClipboard::default());
        let state = AppState::new(
            Paths::new(tmp.path().join("data")),
            TEST_KDF,
            clipboard,
            Box::new(FakeHello::default()),
        );
        state.create_vault(PW).unwrap();
        state
            .edit(|p, ts| {
                let mut e = Entry::new("Example", ts);
                e.username = "alice".into();
                e.password = "fake-pass-1".into();
                e.notes = "multi\nline".into();
                p.entries.push(e);
                let mut gone = Entry::new("Deleted", ts);
                gone.deleted = true;
                p.entries.push(gone);
                Ok(())
            })
            .unwrap();
        (tmp, state)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn csv_export_writes_only_the_target_file() {
        let (tmp, state) = unlocked();
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        let path = out.join("export.csv");
        let result = state.export_to(&path, ExportFormat::Csv, None).unwrap();
        assert_eq!(
            result,
            ExportResult {
                file_name: "export.csv".into(),
                count: 1
            }
        );
        assert_eq!(files_in(&out), vec!["export.csv"], "no temp file left");
        let table = decode(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(table.headers, XPASS_HEADERS);
        assert_eq!(table.rows.len(), 1, "tombstones are not exported");
        assert_eq!(table.rows[0].cells[2], "fake-pass-1");
        assert_eq!(table.rows[0].cells[4], "multi\nline");
    }

    #[test]
    fn export_replaces_an_existing_file() {
        let (tmp, state) = unlocked();
        let path = tmp.path().join("export.csv");
        fs::write(&path, "old content").unwrap();
        state.export_to(&path, ExportFormat::Csv, None).unwrap();
        assert!(fs::read_to_string(&path).unwrap().starts_with("title,"));
    }

    #[test]
    fn kdbx_export_opens_with_the_export_password() {
        let (tmp, state) = unlocked();
        let path = tmp.path().join("export.kdbx");
        let result = state.export_to(&path, ExportFormat::Kdbx, Some(EXPORT_PW)).unwrap();
        assert_eq!(result.count, 1);
        let bytes = fs::read(&path).unwrap();
        let db = Database::parse(&bytes, DatabaseKey::new().with_password(EXPORT_PW)).unwrap();
        assert_eq!(db.num_entries(), 1);
        assert!(Database::parse(&bytes, DatabaseKey::new().with_password(PW)).is_err());
    }

    #[test]
    fn kdbx_needs_a_strong_export_password() {
        let (tmp, state) = unlocked();
        let path = tmp.path().join("export.kdbx");
        assert_eq!(
            state.export_to(&path, ExportFormat::Kdbx, None).unwrap_err(),
            AppError::InvalidInput { field: "kdbxPassword" }
        );
        assert_eq!(
            state
                .export_to(&path, ExportFormat::Kdbx, Some("password1"))
                .unwrap_err(),
            AppError::WeakPassword
        );
        assert!(!path.exists());
    }

    #[test]
    fn locked_vault_writes_nothing() {
        let (tmp, state) = unlocked();
        state.lock();
        let path = tmp.path().join("export.csv");
        assert_eq!(
            state.export_to(&path, ExportFormat::Csv, None).unwrap_err(),
            AppError::Locked
        );
        assert!(!path.exists());
    }

    #[test]
    fn missing_folder_fails_without_leftovers() {
        let (tmp, state) = unlocked();
        let path = tmp.path().join("nope").join("export.csv");
        assert!(matches!(
            state.export_to(&path, ExportFormat::Csv, None).unwrap_err(),
            AppError::Io { .. }
        ));
    }

    #[test]
    fn suggested_name_has_the_date() {
        // 2023-11-14T22:13:20Z
        assert_eq!(
            suggested_file_name(ExportFormat::Csv, 1_700_000_000_000),
            "xpass-export-2023-11-14.csv"
        );
        assert_eq!(
            suggested_file_name(ExportFormat::Kdbx, 1_700_000_000_000),
            "xpass-export-2023-11-14.kdbx"
        );
    }
}
