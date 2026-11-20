//! CSV import (v0.5 plan I3–I6, I10).
//!
//! Rust reads and parses the file and keeps the result as a *pending import*; the
//! WebView only gets an [`ImportPreview`] without passwords, notes or custom-field
//! values. The pending import is dropped on cancel, on commit, on lock and after
//! [`IMPORT_TTL`]. The commit is one [`AppState::edit`]: one D2 timestamp, one
//! backup + atomic write, one sync push.
//!
//! The source path never leaves Rust. It is kept (as the "last import") after the
//! commit so the result screen can offer to delete exactly that file.

use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;
use xpass_interop::{
    apply::{apply_import, ImportOptions, ImportSummary},
    convert::{convert, Parsed, RowWarning, SkippedRow},
    decode::{decode, Table, MAX_FILE_BYTES},
    dedup::{classify, Duplicate},
    preset::{detect, Mapping, Preset},
    text::host_of,
};

use super::*;

/// A pending import is dropped this long after the file was opened.
pub const IMPORT_TTL: Duration = Duration::from_secs(10 * 60);

/// The parsed file, held until commit / cancel / lock / timeout. Zeroized on drop
/// (`Table` and `ImportRow` are).
pub(super) struct PendingImport {
    source: PathBuf,
    file_name: String,
    detected: Preset,
    table: Table,
    parsed: Parsed,
    opened: Instant,
}

/// Pending import plus the source of the last committed one (for "Delete the
/// source CSV").
#[derive(Default)]
pub(super) struct ImportSlot {
    pending: Option<PendingImport>,
    last_source: Option<PathBuf>,
}

/// What the import wizard shows. No passwords, notes or field values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    /// File name without the folder.
    pub file_name: String,
    /// Preset detected from the header row.
    pub detected: Preset,
    /// Preset used for `rows`.
    pub preset: Preset,
    /// Generic preset only: column names of the header row, for the mapping table.
    /// Empty for the other presets, whose columns are known (a file without a header
    /// row would otherwise put its first data row here).
    pub headers: Vec<String>,
    /// Generic preset only: the mapping used (given or guessed).
    pub mapping: Option<Mapping>,
    pub rows: Vec<PreviewRow>,
    /// Rows left out by the parser (empty, unsupported Bitwarden type).
    pub skipped: Vec<SkippedRow>,
    pub counts: PreviewCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    /// Index used in `ImportOptions::excluded`.
    pub index: usize,
    /// Line in the file where the record starts.
    pub line: u64,
    pub title: String,
    pub username: String,
    pub url_host: Option<String>,
    pub has_password: bool,
    pub duplicate: DuplicateKind,
    pub warnings: Vec<RowWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateKind {
    None,
    Exact,
    Possible,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCounts {
    pub new: usize,
    pub exact: usize,
    pub possible: usize,
}

impl AppState {
    /// Reads and parses `path` (chosen in the open dialog) and makes it the pending
    /// import, replacing any earlier one. `preset` overrides the detected one.
    pub fn import_file(&self, path: PathBuf, preset: Option<Preset>) -> AppResult<ImportPreview> {
        if self.status() != VaultStatus::Unlocked {
            return Err(AppError::Locked);
        }
        let bytes = read_limited(&path)?;
        let table = decode(&bytes)?;
        drop(bytes);
        let detected = detect(&table.headers);
        let parsed = convert(&table, preset.unwrap_or(detected), None)?;
        let pending = PendingImport {
            file_name: display_name(&path),
            source: path,
            detected,
            table,
            parsed,
            opened: Instant::now(),
        };

        // Stored under the session lock (lock order: session, then import), so a
        // lock running at the same time cannot leave a pending import behind.
        let guard = lock(&self.0.session);
        let session = guard.as_ref().ok_or(AppError::Locked)?;
        let preview = preview(&session.vault().payload, &pending);
        lock(&self.0.import).pending = Some(pending);
        Ok(preview)
    }

    /// Re-reads the pending file with another preset (and, for the generic preset,
    /// a column mapping; `None` guesses it).
    pub fn import_set_mapping(&self, preset: Preset, mapping: Option<Mapping>) -> AppResult<ImportPreview> {
        let guard = lock(&self.0.session);
        let session = guard.as_ref().ok_or(AppError::Locked)?;
        let mut slot = lock(&self.0.import);
        let pending = live_pending(&mut slot)?;
        let mapping = mapping.filter(|_| preset == Preset::Generic);
        pending.parsed = convert(&pending.table, preset, mapping.as_ref())?;
        Ok(preview(&session.vault().payload, pending))
    }

    /// Writes the pending rows into the vault as one edit and drops them.
    pub fn import_commit(&self, options: &ImportOptions) -> AppResult<ImportSummary> {
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        let mut slot = lock(&self.0.import);
        let pending = live_pending(&mut slot)?;
        if options.excluded.iter().any(|&i| i >= pending.parsed.rows.len()) {
            return Err(AppError::InvalidInput { field: "excluded" });
        }
        let rows = &pending.parsed.rows;
        let summary = session.apply(&self.0.storage, now_ms(), |payload, ts| {
            Ok(apply_import(payload, rows, options, ts))
        })?;
        if summary.added + summary.updated > 0 {
            self.changed(false);
        }
        let done = slot.pending.take().map(|p| p.source.clone());
        slot.last_source = done;
        Ok(summary)
    }

    /// Drops the pending import (the file is left alone).
    pub fn import_cancel(&self) {
        lock(&self.0.import).pending = None;
    }

    /// Deletes the CSV file of the pending or last committed import (plan I10). Only
    /// a file this app opened can be deleted; the UI asks for confirmation first.
    pub fn import_delete_source(&self) -> AppResult<()> {
        if self.status() != VaultStatus::Unlocked {
            return Err(AppError::Locked);
        }
        let mut slot = lock(&self.0.import);
        let path = match (&slot.pending, &slot.last_source) {
            (Some(p), _) => p.source.clone(),
            (None, Some(last)) => last.clone(),
            (None, None) => return Err(AppError::NotFound),
        };
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        slot.last_source = None;
        Ok(())
    }

    /// Drops a pending import opened more than [`IMPORT_TTL`] before `now` (called
    /// by the idle watcher, so the parsed rows do not stay in memory).
    pub fn expire_import(&self, now: Instant) {
        let mut slot = lock(&self.0.import);
        if slot
            .pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.opened) >= IMPORT_TTL)
        {
            slot.pending = None;
        }
    }

    /// Called on lock: the parsed rows and the remembered source go away.
    pub(super) fn clear_import(&self) {
        *lock(&self.0.import) = ImportSlot::default();
    }
}

/// The pending import if there is one that has not timed out.
fn live_pending(slot: &mut ImportSlot) -> AppResult<&mut PendingImport> {
    if slot.pending.as_ref().is_some_and(|p| p.opened.elapsed() >= IMPORT_TTL) {
        slot.pending = None;
    }
    slot.pending.as_mut().ok_or(AppError::ImportExpired)
}

/// Reads at most `MAX_FILE_BYTES + 1` bytes, so a huge file is refused without
/// loading it.
fn read_limited(path: &Path) -> AppResult<Zeroizing<Vec<u8>>> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES as u64 {
        return Err(xpass_interop::ImportError::TooLarge {
            max_bytes: MAX_FILE_BYTES,
        }
        .into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn preview(payload: &Payload, pending: &PendingImport) -> ImportPreview {
    let parsed = &pending.parsed;
    let duplicates = classify(payload, &parsed.rows, &BTreeSet::new());
    let mut counts = PreviewCounts::default();
    let rows = parsed
        .rows
        .iter()
        .zip(duplicates)
        .enumerate()
        .map(|(index, (row, dup))| {
            let duplicate = match dup {
                Duplicate::None => {
                    counts.new += 1;
                    DuplicateKind::None
                }
                Duplicate::Exact { .. } => {
                    counts.exact += 1;
                    DuplicateKind::Exact
                }
                Duplicate::Possible { .. } => {
                    counts.possible += 1;
                    DuplicateKind::Possible
                }
            };
            PreviewRow {
                index,
                line: row.line,
                title: row.title.clone(),
                username: row.username.clone(),
                url_host: row.urls.first().and_then(|u| host_of(u)),
                has_password: !row.password.is_empty(),
                duplicate,
                warnings: row.warnings.clone(),
            }
        })
        .collect();
    ImportPreview {
        file_name: pending.file_name.clone(),
        detected: pending.detected,
        preset: parsed.preset,
        headers: if parsed.preset == Preset::Generic {
            pending.table.headers.clone()
        } else {
            Vec::new()
        },
        mapping: parsed.mapping.clone(),
        rows,
        skipped: parsed.skipped.clone(),
        counts,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use xpass_core::model::{Entry, HISTORY_NOTE_IMPORT};
    use xpass_interop::apply::DuplicateChoice;

    use super::*;
    use crate::{hello::fake::FakeHello, paths::Paths};

    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";
    /// Synthetic Chrome export (fake sites and passwords only).
    const CHROME: &str = "name,url,username,password,note\n\
        Example,https://example.com/login,alice,fake-pass-1,\n\
        ,https://www.example.org/,bob,fake-pass-2,a note\n";

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
        (tmp, state)
    }

    fn csv_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn backups(state: &AppState) -> usize {
        state.list_backups().unwrap().len()
    }

    #[test]
    fn preview_has_no_secrets_and_detects_the_preset() {
        let (tmp, state) = unlocked();
        let path = csv_file(&tmp, "chrome.csv", CHROME);
        let preview = state.import_file(path, None).unwrap();
        assert_eq!(preview.file_name, "chrome.csv");
        assert_eq!(preview.detected, Preset::Chrome);
        assert_eq!(preview.preset, Preset::Chrome);
        assert_eq!(
            preview.counts,
            PreviewCounts {
                new: 2,
                exact: 0,
                possible: 0
            }
        );
        assert_eq!(preview.rows[1].title, "example.org");
        assert_eq!(preview.rows[1].url_host.as_deref(), Some("example.org"));
        assert!(preview.rows[0].has_password);
        assert!(preview.headers.is_empty(), "headers only for the generic preset");
        let json = serde_json::to_string(&preview).unwrap();
        assert!(!json.contains("fake-pass"), "no password in the preview");
        assert!(!json.contains("a note"), "no notes in the preview");
        assert!(!json.contains(tmp.path().to_str().unwrap()), "no folder in the preview");
    }

    #[test]
    fn commit_is_one_edit_with_one_backup_and_one_sync_trigger() {
        let (tmp, state) = unlocked();
        let triggers = Arc::new(StdMutex::new(Vec::new()));
        let seen = triggers.clone();
        state.set_sync_notifier(move |t| seen.lock().unwrap().push(t));
        let before = backups(&state);

        state.import_file(csv_file(&tmp, "c.csv", CHROME), None).unwrap();
        let summary = state.import_commit(&ImportOptions::default()).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                added: 2,
                updated: 0,
                skipped: 0
            }
        );
        assert_eq!(backups(&state), before + 1, "one write → one rolling backup");
        assert_eq!(*triggers.lock().unwrap(), vec![Trigger::Edited], "one sync push");
        let (ts, count) = state
            .read(|p| {
                Ok((
                    p.entries.iter().map(|e| e.updated_at).collect::<BTreeSet<_>>(),
                    p.entries.len(),
                ))
            })
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(ts.len(), 1, "one timestamp for the whole import");

        // The pending import is gone; a second import of the same file adds nothing.
        assert_eq!(
            state.import_commit(&ImportOptions::default()).unwrap_err(),
            AppError::ImportExpired
        );
        let again = state.import_file(csv_file(&tmp, "c.csv", CHROME), None).unwrap();
        assert_eq!(
            again.counts,
            PreviewCounts {
                new: 0,
                exact: 2,
                possible: 0
            }
        );
        let summary = state.import_commit(&ImportOptions::default()).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                added: 0,
                updated: 0,
                skipped: 2
            }
        );
        assert_eq!(triggers.lock().unwrap().len(), 1, "nothing changed → no push");
    }

    #[test]
    fn possible_duplicate_update_stamps_history() {
        let (tmp, state) = unlocked();
        state
            .edit(|p, ts| {
                let mut e = Entry::new("Example", ts);
                e.username = "alice".into();
                e.password = "old-fake-pass".into();
                e.urls = vec!["https://example.com".into()];
                p.entries.push(e);
                Ok(())
            })
            .unwrap();
        let preview = state.import_file(csv_file(&tmp, "c.csv", CHROME), None).unwrap();
        assert_eq!(preview.rows[0].duplicate, DuplicateKind::Possible);

        let options = ImportOptions {
            duplicates: DuplicateChoice::Update,
            excluded: BTreeSet::from([1]),
        };
        let summary = state.import_commit(&options).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                added: 0,
                updated: 1,
                skipped: 1
            }
        );
        state
            .read(|p| {
                assert_eq!(p.entries.len(), 1);
                let e = &p.entries[0];
                assert_eq!(e.password, "fake-pass-1");
                assert_eq!(e.history.len(), 1);
                assert_eq!(e.history[0].password, "old-fake-pass");
                assert_eq!(e.history[0].note.as_deref(), Some(HISTORY_NOTE_IMPORT));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn lock_drops_the_pending_import() {
        let (tmp, state) = unlocked();
        let path = csv_file(&tmp, "c.csv", CHROME);
        state.import_file(path.clone(), None).unwrap();
        state.lock();
        assert!(lock(&state.0.import).pending.is_none());
        state.unlock_password(PW).unwrap();
        assert_eq!(
            state.import_commit(&ImportOptions::default()).unwrap_err(),
            AppError::ImportExpired
        );
        assert_eq!(state.import_delete_source().unwrap_err(), AppError::NotFound);
        assert!(path.exists());
        state.import_file(path, None).unwrap();
        state.import_commit(&ImportOptions::default()).unwrap();
    }

    #[test]
    fn import_needs_an_unlocked_vault() {
        let (tmp, state) = unlocked();
        state.lock();
        let path = csv_file(&tmp, "c.csv", CHROME);
        assert_eq!(state.import_file(path, None).unwrap_err(), AppError::Locked);
        assert_eq!(
            state.import_commit(&ImportOptions::default()).unwrap_err(),
            AppError::Locked
        );
    }

    #[test]
    fn pending_import_expires() {
        let (tmp, state) = unlocked();
        state.import_file(csv_file(&tmp, "c.csv", CHROME), None).unwrap();
        state.expire_import(Instant::now());
        assert!(lock(&state.0.import).pending.is_some());
        state.expire_import(Instant::now() + IMPORT_TTL);
        assert_eq!(
            state.import_set_mapping(Preset::Generic, None).unwrap_err(),
            AppError::ImportExpired
        );
    }

    #[test]
    fn cancel_drops_and_keeps_the_file() {
        let (tmp, state) = unlocked();
        let path = csv_file(&tmp, "c.csv", CHROME);
        state.import_file(path.clone(), None).unwrap();
        state.import_cancel();
        assert_eq!(
            state.import_commit(&ImportOptions::default()).unwrap_err(),
            AppError::ImportExpired
        );
        assert!(path.exists());
    }

    #[test]
    fn delete_source_removes_only_the_imported_file() {
        let (tmp, state) = unlocked();
        let other = csv_file(&tmp, "other.csv", CHROME);
        let path = csv_file(&tmp, "c.csv", CHROME);
        assert_eq!(state.import_delete_source().unwrap_err(), AppError::NotFound);
        state.import_file(path.clone(), None).unwrap();
        state.import_commit(&ImportOptions::default()).unwrap();
        state.import_delete_source().unwrap();
        assert!(!path.exists());
        assert!(other.exists());
        assert_eq!(state.import_delete_source().unwrap_err(), AppError::NotFound);
    }

    #[test]
    fn generic_mapping_and_preset_override() {
        let (tmp, state) = unlocked();
        let path = csv_file(
            &tmp,
            "g.csv",
            "Site;Login;Secret;Comment\nExample;alice;fake-pass-3;hello\n",
        );
        let preview = state.import_file(path, None).unwrap();
        assert_eq!(preview.preset, Preset::Generic);
        let guessed = preview.mapping.clone().unwrap();
        assert_eq!(guessed.username, Some(1));
        assert_eq!(preview.headers, vec!["Site", "Login", "Secret", "Comment"]);

        let mapping = Mapping {
            title: Some(3),
            ..guessed
        };
        let preview = state.import_set_mapping(Preset::Generic, Some(mapping)).unwrap();
        assert_eq!(preview.rows[0].title, "hello");

        let bad = Mapping {
            title: Some(9),
            ..Mapping::default()
        };
        assert!(matches!(
            state.import_set_mapping(Preset::Generic, Some(bad)).unwrap_err(),
            AppError::InvalidInput { .. }
        ));
        // A failed re-map keeps the pending import.
        assert!(state.import_set_mapping(Preset::Generic, None).is_ok());
    }

    #[test]
    fn bad_files_are_refused_with_clear_codes() {
        let (tmp, state) = unlocked();
        let latin1 = tmp.path().join("latin1.csv");
        fs::write(&latin1, b"name,url,username,password,note\nCaf\xe9,,,x,\n").unwrap();
        assert_eq!(state.import_file(latin1, None).unwrap_err(), AppError::ImportEncoding);

        let empty = csv_file(&tmp, "empty.csv", "name,url,username,password,note\n");
        assert_eq!(state.import_file(empty, None).unwrap_err(), AppError::ImportEmpty);

        let big = tmp.path().join("big.csv");
        let file = fs::File::create(&big).unwrap();
        file.set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
        assert!(matches!(
            state.import_file(big, None).unwrap_err(),
            AppError::ImportTooLarge { .. }
        ));

        assert!(matches!(
            state.import_file(tmp.path().join("missing.csv"), None).unwrap_err(),
            AppError::Io { .. }
        ));
    }

    #[test]
    fn excluded_index_out_of_range_is_refused() {
        let (tmp, state) = unlocked();
        state.import_file(csv_file(&tmp, "c.csv", CHROME), None).unwrap();
        let options = ImportOptions {
            excluded: BTreeSet::from([7]),
            ..ImportOptions::default()
        };
        assert_eq!(
            state.import_commit(&options).unwrap_err(),
            AppError::InvalidInput { field: "excluded" }
        );
    }
}
