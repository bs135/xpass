//! Restoring a previous version from the local backups (T22, decision S14).
//!
//! Every write keeps a copy of the replaced file in `backups/`. A restore does not
//! put the old file back: core `restore` rewrites the entries that differ with a
//! fresh timestamp, so the restored state is an ordinary edit that wins the next
//! sync on every device. The current file goes to the backups like on any write.

use std::{fs, path::Path};

use xpass_core::restore::restore;

use super::*;

/// Whether a backup can be restored, as seen with the key in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupKind {
    /// Opens with the current key.
    Current,
    /// Saved before a VaultKey rotation: needs the master password it was saved with.
    OlderKey,
    /// Belongs to another vault (e.g. before "replace with the vault on GitHub").
    OtherVault,
    /// Damaged or not a vault file.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// File name in `backups/`; the id for `restore_backup`.
    pub name: String,
    /// Unix ms when this version was written (file time), if known.
    pub saved_at: Option<i64>,
    pub size: u64,
    pub kind: BackupKind,
    /// Entries (not deleted) when it opens with the current key.
    pub entries: Option<usize>,
}

impl AppState {
    /// Local backups, newest first. Needs the vault unlocked (they are checked with
    /// the key in memory; no password is needed for that).
    pub fn list_backups(&self) -> AppResult<Vec<BackupInfo>> {
        let guard = lock(&self.0.session);
        let session = guard.as_ref().ok_or(AppError::Locked)?;
        let mut out = Vec::new();
        for path in self.0.storage.backups()?.into_iter().rev() {
            let Some(name) = file_name(&path) else {
                continue;
            };
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let saved_at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok());
            let (kind, entries) = match fs::read(&path).map(|bytes| session.vault().open_sibling(&bytes)) {
                Ok(Ok(backup)) => (BackupKind::Current, Some(live_entries(&backup.payload))),
                Ok(Err(xpass_core::Error::KeyChanged)) => (BackupKind::OlderKey, None),
                Ok(Err(xpass_core::Error::DifferentVault)) => (BackupKind::OtherVault, None),
                _ => (BackupKind::Unreadable, None),
            };
            out.push(BackupInfo {
                name,
                saved_at,
                size: meta.len(),
                kind,
                entries,
            });
        }
        Ok(out)
    }

    /// Restores the backup `name` as a new edit (S14). `password` is needed only for
    /// a backup saved before a key rotation (the master password of that time;
    /// throttled like unlock). Backups of another vault are refused.
    pub fn restore_backup(&self, name: &str, password: Option<&str>) -> AppResult<()> {
        let _gate = lock(&self.0.gate);
        if self.status() != VaultStatus::Unlocked {
            return Err(AppError::Locked);
        }
        let path = self
            .0
            .storage
            .backups()?
            .into_iter()
            .find(|p| file_name(p).as_deref() == Some(name))
            .ok_or(AppError::NotFound)?;
        let bytes = fs::read(&path)?;
        // Argon2 runs before the session lock is taken.
        let unlocked = match password {
            Some(password) => Some(self.open_with_password(&bytes, password)?),
            None => None,
        };

        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        let backup = match unlocked {
            Some(backup) if backup.vault_id() != session.vault().vault_id() => return Err(AppError::DifferentVault),
            Some(backup) => backup,
            None => session.vault().open_sibling(&bytes)?,
        };
        session.apply(&self.0.storage, now_ms(), |payload, ts| {
            *payload = restore(payload, &backup.payload, ts);
            Ok(())
        })?;
        self.changed(false);
        Ok(())
    }

    fn open_with_password(&self, bytes: &[u8], password: &str) -> AppResult<UnlockedVault> {
        self.check_throttle()?;
        match UnlockedVault::unlock(bytes, Unlock::Password(password.as_bytes())) {
            Ok(vault) => {
                lock(&self.0.throttle).record_success();
                Ok(vault)
            }
            Err(xpass_core::Error::BadCredentials) => {
                lock(&self.0.throttle).record_failure(Instant::now());
                Err(AppError::BadCredentials)
            }
            Err(e) => Err(e.into()),
        }
    }
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().and_then(|n| n.to_str()).map(str::to_owned)
}

fn live_entries(payload: &Payload) -> usize {
    payload.entries.iter().filter(|e| !e.deleted).count()
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use xpass_core::model::Entry;

    use super::*;
    use crate::{clipboard::fake::FakeClipboard, hello::fake::FakeHello};

    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";
    const WRONG: &str = "not the password at all";

    fn created() -> (tempfile::TempDir, Paths, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("data"));
        let state = AppState::new(
            paths.clone(),
            TEST_KDF,
            SecureClipboard::new(FakeClipboard::default()),
            Box::new(FakeHello::default()),
        );
        state.create_vault(PW).unwrap();
        (tmp, paths, state)
    }

    /// Backup names have millisecond precision: keep writes apart.
    fn pause() {
        thread::sleep(Duration::from_millis(3));
    }

    fn add(state: &AppState, title: &str) {
        pause();
        state
            .edit(|p, ts| {
                p.entries.push(Entry::new(title, ts));
                Ok(())
            })
            .unwrap();
    }

    fn titles(state: &AppState) -> Vec<String> {
        let mut t: Vec<String> = state
            .read(|p| {
                Ok(p.entries
                    .iter()
                    .filter(|e| !e.deleted)
                    .map(|e| e.title.clone())
                    .collect())
            })
            .unwrap();
        t.sort();
        t
    }

    fn kinds(state: &AppState) -> Vec<(BackupKind, Option<usize>)> {
        state
            .list_backups()
            .unwrap()
            .iter()
            .map(|b| (b.kind, b.entries))
            .collect()
    }

    #[test]
    fn restore_brings_back_an_earlier_state_as_a_new_edit() {
        let (_tmp, _paths, state) = created();
        add(&state, "Mail");
        add(&state, "Bank");
        assert_eq!(
            kinds(&state),
            [(BackupKind::Current, Some(1)), (BackupKind::Current, Some(0))],
            "newest first"
        );
        let before_bank = state.list_backups().unwrap()[0].clone();
        assert!(before_bank.saved_at.is_some() && before_bank.size > 0);

        pause();
        state.restore_backup(&before_bank.name, None).unwrap();
        assert_eq!(titles(&state), ["Mail"]);
        let (tombstones, newest) = state
            .read(|p| {
                let newest = p.entries.iter().map(|e| e.updated_at).max().unwrap();
                let tombstones: Vec<i64> = p.entries.iter().filter(|e| e.deleted).map(|e| e.updated_at).collect();
                Ok((tombstones, newest))
            })
            .unwrap();
        assert_eq!(
            tombstones,
            [newest],
            "Bank deleted with the newest timestamp, so it syncs"
        );
        assert_eq!(state.list_backups().unwrap().len(), 3, "the replaced version is kept");

        state.lock();
        state.unlock_password(PW).unwrap();
        assert_eq!(titles(&state), ["Mail"]);

        for name in ["vault-nope.xpv", "../vault.xpv", "vault.xpv"] {
            assert_eq!(state.restore_backup(name, None), Err(AppError::NotFound));
        }
        state.lock();
        assert_eq!(state.list_backups(), Err(AppError::Locked));
        assert_eq!(state.restore_backup(&before_bank.name, None), Err(AppError::Locked));
    }

    #[test]
    fn a_backup_from_before_a_rotation_needs_its_password() {
        let (_tmp, _paths, state) = created();
        add(&state, "Mail");
        pause();
        state.rotate_vault_key(PW).unwrap();
        add(&state, "Later");
        assert_eq!(
            kinds(&state)[..2],
            [(BackupKind::Current, Some(1)), (BackupKind::OlderKey, None)]
        );
        let old = state.list_backups().unwrap()[1].name.clone();

        assert_eq!(state.restore_backup(&old, None), Err(AppError::KeyChanged));
        assert_eq!(state.restore_backup(&old, Some(WRONG)), Err(AppError::BadCredentials));
        pause();
        state.restore_backup(&old, Some(PW)).unwrap();
        assert_eq!(titles(&state), ["Mail"]);

        // Only the entries came back: the file keeps the new key.
        state.lock();
        state.unlock_password(PW).unwrap();
        assert_eq!(titles(&state), ["Mail"]);
    }

    #[test]
    fn a_backup_of_another_vault_is_refused() {
        let (_tmp, paths, state) = created();
        add(&state, "Mail");
        let (_tmp2, other_paths, _other) = created();
        let foreign = "vault-20000101-000000-000.xpv";
        fs::copy(other_paths.vault(), paths.backups().join(foreign)).unwrap();
        fs::write(paths.backups().join("vault-20000101-000000-001.xpv"), b"garbage").unwrap();

        let list = state.list_backups().unwrap();
        let kind = |name: &str| list.iter().find(|b| b.name == name).unwrap().kind;
        assert_eq!(kind(foreign), BackupKind::OtherVault);
        assert_eq!(kind("vault-20000101-000000-001.xpv"), BackupKind::Unreadable);

        assert_eq!(state.restore_backup(foreign, None), Err(AppError::DifferentVault));
        assert_eq!(state.restore_backup(foreign, Some(PW)), Err(AppError::DifferentVault));
        assert_eq!(titles(&state), ["Mail"]);
    }
}
