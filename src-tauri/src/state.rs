//! App state and the vault lifecycle (create, unlock, lock, change password, new kit).
//!
//! Plain Rust, no Tauri types, so the command handlers can be tested with a temp
//! directory. The Tauri commands in `commands::session` are thin wrappers.

use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use rand::rngs::OsRng;
use serde::Serialize;
use xpass_core::{crypto::KdfParams, model::Payload, Unlock, UnlockedVault};
use zeroize::Zeroizing;

use crate::{
    clipboard::{SecureClipboard, DEFAULT_CLEAR_AFTER},
    entries::{self, FieldRef},
    error::{AppError, AppResult},
    paths::Paths,
    session::VaultSession,
    storage::Storage,
    strength,
    throttle::Throttle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultStatus {
    NoVault,
    Locked,
    Unlocked,
}

/// Shared app state, cheap to clone (moved into `spawn_blocking` for Argon2).
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

struct Inner {
    storage: Storage,
    /// KDF for new and re-wrapped password slots (`KdfParams::DEFAULT` in the app).
    target_kdf: KdfParams,
    session: Mutex<Option<VaultSession>>,
    throttle: Mutex<Throttle>,
    /// Serializes create/unlock so concurrent attempts cannot bypass the throttle.
    gate: Mutex<()>,
    clipboard: SecureClipboard,
}

impl AppState {
    pub fn new(paths: Paths, target_kdf: KdfParams, clipboard: SecureClipboard) -> Self {
        Self(Arc::new(Inner {
            storage: Storage::new(paths),
            target_kdf,
            session: Mutex::new(None),
            throttle: Mutex::new(Throttle::default()),
            gate: Mutex::new(()),
            clipboard,
        }))
    }

    pub fn status(&self) -> VaultStatus {
        if lock(&self.0.session).is_some() {
            VaultStatus::Unlocked
        } else if self.0.storage.exists() {
            VaultStatus::Locked
        } else {
            VaultStatus::NoVault
        }
    }

    /// Creates the vault and unlocks it. Returns the printable recovery key, which
    /// must be shown once (Emergency Kit) and is never stored.
    pub fn create_vault(&self, password: &str) -> AppResult<Zeroizing<String>> {
        let _gate = lock(&self.0.gate);
        if self.0.storage.exists() {
            return Err(AppError::VaultExists);
        }
        if !strength::is_strong_enough(password) {
            return Err(AppError::WeakPassword);
        }
        let (mut vault, recovery_key) = UnlockedVault::create(&mut OsRng, password.as_bytes(), self.0.target_kdf)?;
        let bytes = vault.save(&mut OsRng)?;
        self.0.storage.write(&bytes, now_ms())?;
        *lock(&self.0.session) = Some(VaultSession::new(vault, false));
        Ok(recovery_key)
    }

    pub fn unlock_password(&self, password: &str) -> AppResult<()> {
        self.unlock(Unlock::Password(password.as_bytes()), Some(password))
    }

    pub fn unlock_recovery(&self, recovery_key: &str) -> AppResult<()> {
        self.unlock(Unlock::Recovery(recovery_key), None)
    }

    /// Sets a new master password. `current` is required unless the session was
    /// unlocked with the recovery key (forgotten password).
    pub fn change_password(&self, current: Option<&str>, new: &str) -> AppResult<()> {
        let _gate = lock(&self.0.gate);
        if !strength::is_strong_enough(new) {
            return Err(AppError::WeakPassword);
        }
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        self.authorize(session, current)?;
        let target = self.0.target_kdf;
        session.persist_with(&self.0.storage, now_ms(), |vault| {
            vault
                .change_password(&mut OsRng, new.as_bytes(), target)
                .map_err(AppError::from)
        })
    }

    /// Replaces the recovery key (new Emergency Kit) and returns it. `current` is
    /// required unless the session was unlocked with the recovery key.
    pub fn new_recovery_key(&self, current: Option<&str>) -> AppResult<Zeroizing<String>> {
        let _gate = lock(&self.0.gate);
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        self.authorize(session, current)?;
        let mut recovery_key = None;
        session.persist_with(&self.0.storage, now_ms(), |vault| {
            recovery_key = Some(vault.replace_recovery_key(&mut OsRng));
            Ok(())
        })?;
        recovery_key.ok_or(AppError::Internal)
    }

    /// Checks the current password for a sensitive action, with the same throttle
    /// as unlock. Skipped right after a recovery-key unlock.
    fn authorize(&self, session: &VaultSession, current: Option<&str>) -> AppResult<()> {
        if session.recovered() {
            return Ok(());
        }
        let current = current.ok_or(AppError::InvalidInput {
            field: "currentPassword",
        })?;
        self.check_throttle()?;
        match session.vault().check_password(current.as_bytes()) {
            Ok(()) => {
                lock(&self.0.throttle).record_success();
                Ok(())
            }
            Err(xpass_core::Error::BadCredentials) => {
                lock(&self.0.throttle).record_failure(Instant::now());
                Err(AppError::BadCredentials)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn check_throttle(&self) -> AppResult<()> {
        lock(&self.0.throttle)
            .check(Instant::now())
            .map_err(|wait| AppError::Throttled {
                retry_after_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX).max(1),
            })
    }

    /// Runs `f` on the unlocked payload (`Locked` otherwise).
    pub fn read<T>(&self, f: impl FnOnce(&Payload) -> AppResult<T>) -> AppResult<T> {
        let guard = lock(&self.0.session);
        let session = guard.as_ref().ok_or(AppError::Locked)?;
        f(&session.vault().payload)
    }

    /// Applies `f` to the payload with a D2 timestamp, then saves and writes the
    /// vault atomically. Nothing changes if `f` or the write fails.
    pub fn edit<T>(&self, f: impl FnOnce(&mut Payload, i64) -> AppResult<T>) -> AppResult<T> {
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        session.apply(&self.0.storage, now_ms(), f)
    }

    /// Drops the session (wiping keys and secrets). Returns whether it was unlocked.
    /// Also clears the clipboard if it still holds a secret we copied.
    pub fn lock(&self) -> bool {
        self.0.clipboard.clear_if_ours();
        lock(&self.0.session).take().is_some()
    }

    /// Copies an entry field to the clipboard; it is cleared after the timeout.
    pub fn copy_field(&self, id: uuid::Uuid, field: FieldRef) -> AppResult<()> {
        let value = self.read(|p| entries::field_value(p, id, field))?;
        self.0.clipboard.copy(&value, DEFAULT_CLEAR_AFTER)
    }

    /// Copies arbitrary secret text (e.g. a generated password) the same secure way.
    pub fn copy_text(&self, text: &str) -> AppResult<()> {
        if text.is_empty() {
            return Err(AppError::InvalidInput { field: "text" });
        }
        self.0.clipboard.copy(text, DEFAULT_CLEAR_AFTER)
    }

    /// Returns the password or a secret custom field for display.
    pub fn reveal_secret(&self, id: uuid::Uuid, field: FieldRef) -> AppResult<Zeroizing<String>> {
        self.read(|p| entries::secret_value(p, id, field))
    }

    fn unlock(&self, how: Unlock<'_>, password: Option<&str>) -> AppResult<()> {
        let _gate = lock(&self.0.gate);
        if lock(&self.0.session).is_some() {
            return Ok(());
        }
        self.check_throttle()?;
        let file = self.0.storage.read()?.ok_or(AppError::NoVault)?;

        let vault = match UnlockedVault::unlock(&file, how) {
            Ok(vault) => vault,
            Err(xpass_core::Error::BadCredentials) => {
                lock(&self.0.throttle).record_failure(Instant::now());
                return Err(AppError::BadCredentials);
            }
            // A recovery-key typo is caught by its checksum and is not a guess.
            Err(e) => return Err(e.into()),
        };
        lock(&self.0.throttle).record_success();

        let mut session = VaultSession::new(vault, password.is_none());
        if let Some(password) = password {
            self.rewrap_if_weak(&mut session, password);
        }
        *lock(&self.0.session) = Some(session);
        Ok(())
    }

    /// D6: re-wraps a password slot created with weaker KDF parameters. Best effort:
    /// the vault stays usable with the old slot if this fails.
    fn rewrap_if_weak(&self, session: &mut VaultSession, password: &str) {
        let target = self.0.target_kdf;
        let weak = session
            .vault()
            .password_kdf()
            .is_some_and(|kdf| kdf.is_weaker_than(&target));
        if !weak {
            return;
        }
        let result = session.persist_with(&self.0.storage, now_ms(), |vault| {
            vault
                .change_password(&mut OsRng, password.as_bytes(), target)
                .map_err(AppError::from)
        });
        if let Err(e) = result {
            eprintln!("xpass: re-wrapping the password slot failed: {e}");
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Locks a mutex, recovering from poisoning (a panic elsewhere must not brick the app).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use xpass_core::{
        format::{self, SlotParams},
        model::Entry,
    };

    use super::*;

    /// Smallest parameters `KdfParams::validate` accepts, to keep tests fast.
    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";

    fn test_clipboard() -> SecureClipboard {
        SecureClipboard::new(crate::clipboard::fake::FakeClipboard::default())
    }

    fn setup() -> (tempfile::TempDir, Paths, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("data"));
        let state = AppState::new(paths.clone(), TEST_KDF, test_clipboard());
        (tmp, paths, state)
    }

    fn created() -> (tempfile::TempDir, Paths, AppState, String) {
        let (tmp, paths, state) = setup();
        let rec = state.create_vault(PW).unwrap().to_string();
        (tmp, paths, state, rec)
    }

    fn password_kdf_on_disk(paths: &Paths) -> KdfParams {
        let file = fs::read(paths.vault()).unwrap();
        let parsed = format::parse(&file).unwrap();
        parsed
            .header
            .slots
            .iter()
            .find_map(|s| match s.params {
                SlotParams::Password { kdf, .. } => Some(kdf),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn create_lock_unlock_roundtrip() {
        let (_tmp, paths, state) = setup();
        assert_eq!(state.status(), VaultStatus::NoVault);

        let rec = state.create_vault(PW).unwrap();
        assert_eq!(rec.len(), 65, "55 base32 chars + 10 dashes");
        assert_eq!(state.status(), VaultStatus::Unlocked);
        assert!(paths.vault().is_file());

        assert!(state.lock());
        assert!(!state.lock(), "second lock is a no-op");
        assert_eq!(state.status(), VaultStatus::Locked);

        state.unlock_password(PW).unwrap();
        assert_eq!(state.status(), VaultStatus::Unlocked);
    }

    #[test]
    fn new_app_instance_sees_a_locked_vault() {
        let (_tmp, paths, _state, _) = created();
        let restarted = AppState::new(paths, TEST_KDF, test_clipboard());
        assert_eq!(restarted.status(), VaultStatus::Locked);
        restarted.unlock_password(PW).unwrap();
    }

    #[test]
    fn create_rejects_existing_vault_and_empty_password() {
        let (_tmp, _paths, state) = setup();
        assert_eq!(state.create_vault("").unwrap_err(), AppError::WeakPassword);
        assert_eq!(state.create_vault("password1").unwrap_err(), AppError::WeakPassword);
        assert_eq!(state.status(), VaultStatus::NoVault);
        state.create_vault(PW).unwrap();
        assert_eq!(state.create_vault(PW).unwrap_err(), AppError::VaultExists);
    }

    #[test]
    fn unlock_without_vault() {
        let (_tmp, _paths, state) = setup();
        assert_eq!(state.unlock_password(PW).unwrap_err(), AppError::NoVault);
    }

    #[test]
    fn wrong_password_is_throttled() {
        let (_tmp, _paths, state, _) = created();
        state.lock();
        assert_eq!(state.unlock_password("nope").unwrap_err(), AppError::BadCredentials);
        assert_eq!(state.unlock_password("nope").unwrap_err(), AppError::BadCredentials);
        // Two failures -> 1 s delay; even the right password must wait.
        match state.unlock_password(PW).unwrap_err() {
            AppError::Throttled { retry_after_ms } => assert!((1..=1000).contains(&retry_after_ms)),
            other => panic!("expected Throttled, got {other:?}"),
        }
        assert_eq!(state.status(), VaultStatus::Locked);
    }

    #[test]
    fn recovery_key_unlocks_and_typos_are_not_throttled() {
        let (_tmp, _paths, state, rec) = created();
        state.lock();
        let mut typo: Vec<char> = rec.chars().collect();
        typo[0] = if typo[0] == 'A' { 'B' } else { 'A' };
        let typo: String = typo.into_iter().collect();
        for _ in 0..3 {
            assert_eq!(state.unlock_recovery(&typo).unwrap_err(), AppError::BadRecoveryKey);
        }
        state.unlock_recovery(&rec.to_lowercase()).unwrap();
        assert_eq!(state.status(), VaultStatus::Unlocked);
    }

    #[test]
    fn tampered_and_garbage_files_are_reported() {
        let (_tmp, paths, state, _) = created();
        state.lock();
        let good = fs::read(paths.vault()).unwrap();

        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 0x01;
        fs::write(paths.vault(), &bad).unwrap();
        assert_eq!(state.unlock_password(PW).unwrap_err(), AppError::Tampered);

        fs::write(paths.vault(), b"not a vault").unwrap();
        assert_eq!(state.unlock_password(PW).unwrap_err(), AppError::InvalidFile);

        fs::write(paths.vault(), &good).unwrap();
        state.unlock_password(PW).unwrap();
    }

    #[test]
    fn weak_password_slot_is_rewrapped_on_unlock() {
        let (_tmp, paths, _state, rec) = created();
        let stronger = KdfParams {
            m_kib: 16 * 1024,
            ..TEST_KDF
        };
        let upgraded = AppState::new(paths.clone(), stronger, test_clipboard());

        upgraded.unlock_recovery(&rec).unwrap();
        assert_eq!(
            password_kdf_on_disk(&paths),
            TEST_KDF,
            "recovery unlock has no password to re-wrap"
        );
        upgraded.lock();

        upgraded.unlock_password(PW).unwrap();
        assert_eq!(password_kdf_on_disk(&paths), stronger);
        upgraded.lock();
        upgraded.unlock_password(PW).unwrap();
    }

    #[test]
    fn session_apply_stamps_with_d2_and_persists() {
        let (_tmp, paths, state, _) = created();
        let storage = Storage::new(paths.clone());
        let mut guard = lock(&state.0.session);
        let session = guard.as_mut().unwrap();

        // The clock is behind: the timestamp must still increase.
        session.last_seen_ts = 5_000;
        session
            .apply(&storage, 1_000, |p, ts| {
                let mut e = Entry::new("GitHub", ts);
                e.password = "s3cret".into();
                p.entries.push(e);
                Ok(())
            })
            .unwrap();
        assert_eq!(session.vault().payload.entries[0].updated_at, 5_001);
        assert_eq!(session.last_seen_ts, 5_001);
        drop(guard);

        state.lock();
        state.unlock_password(PW).unwrap();
        let guard = lock(&state.0.session);
        let v = guard.as_ref().unwrap().vault();
        assert_eq!(v.payload.entries[0].password, "s3cret");
        assert_eq!(v.payload.revision, 2);
        assert_eq!(Storage::new(paths).backups().unwrap().len(), 1);
    }

    #[test]
    fn failed_write_leaves_the_session_unchanged() {
        let (_tmp, paths, state, _) = created();
        // A file where the backups directory should be makes the next write fail.
        fs::write(paths.backups(), b"blocker").unwrap();
        let storage = Storage::new(paths);
        let mut guard = lock(&state.0.session);
        let session = guard.as_mut().unwrap();
        let err = session
            .apply(&storage, 1_000, |p, ts| {
                p.entries.push(Entry::new("x", ts));
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Io { .. }), "{err:?}");
        assert!(session.vault().payload.entries.is_empty());
        assert_eq!(session.vault().payload.revision, 1);
    }

    const NEW_PW: &str = "another long passphrase 2026";

    #[test]
    fn change_password_needs_the_current_one() {
        let (_tmp, _paths, state, rec) = created();
        assert_eq!(
            state.change_password(None, NEW_PW).unwrap_err(),
            AppError::InvalidInput {
                field: "currentPassword"
            }
        );
        assert_eq!(
            state.change_password(Some("wrong"), NEW_PW).unwrap_err(),
            AppError::BadCredentials
        );
        assert_eq!(
            state.change_password(Some(PW), "short").unwrap_err(),
            AppError::WeakPassword
        );
        state.change_password(Some(PW), NEW_PW).unwrap();

        state.lock();
        assert_eq!(state.unlock_password(PW).unwrap_err(), AppError::BadCredentials);
        state.unlock_password(NEW_PW).unwrap();
        state.lock();
        state.unlock_recovery(&rec).unwrap();
    }

    #[test]
    fn recovery_unlock_allows_reset_without_current_password() {
        let (_tmp, _paths, state, old_rec) = created();
        state.lock();
        state.unlock_recovery(&old_rec).unwrap();
        state.change_password(None, NEW_PW).unwrap();
        let new_rec = state.new_recovery_key(None).unwrap();

        state.lock();
        assert_eq!(state.unlock_recovery(&old_rec).unwrap_err(), AppError::BadCredentials);
        state.unlock_recovery(&new_rec).unwrap();
        state.lock();
        state.unlock_password(NEW_PW).unwrap();
        // A normal unlock requires the password again.
        assert!(matches!(
            state.new_recovery_key(None).unwrap_err(),
            AppError::InvalidInput { .. }
        ));
    }

    #[test]
    fn new_recovery_key_with_password() {
        let (_tmp, _paths, state, old_rec) = created();
        let new_rec = state.new_recovery_key(Some(PW)).unwrap();
        assert_ne!(*new_rec, old_rec);
        state.lock();
        state.unlock_recovery(&new_rec).unwrap();
    }

    #[test]
    fn sensitive_actions_need_an_unlocked_vault() {
        let (_tmp, _paths, state, _) = created();
        state.lock();
        assert_eq!(state.change_password(Some(PW), NEW_PW).unwrap_err(), AppError::Locked);
        assert_eq!(state.new_recovery_key(Some(PW)).unwrap_err(), AppError::Locked);
    }

    #[test]
    fn entry_commands_need_an_unlocked_vault_and_persist() {
        use crate::entries::{self, EntryInput};
        let (_tmp, _paths, state, _) = created();
        let input = EntryInput {
            id: None,
            title: "GitHub".into(),
            username: "alice".into(),
            password: Some("gh-secret".into()),
            urls: vec!["github.com".into()],
            notes: String::new(),
            tags: Vec::new(),
            favorite: false,
        };
        let id = state.edit(|p, ts| entries::save(p, &input, ts)).unwrap();
        // A failing edit leaves the vault unchanged.
        assert_eq!(
            state
                .edit(|p, ts| entries::delete(p, uuid::Uuid::nil(), ts))
                .unwrap_err(),
            AppError::NotFound
        );

        state.lock();
        assert_eq!(state.read(|p| Ok(entries::list(p))).unwrap_err(), AppError::Locked);
        assert_eq!(
            state.edit(|p, ts| entries::delete(p, id, ts)).unwrap_err(),
            AppError::Locked
        );

        state.unlock_password(PW).unwrap();
        let rows = state.read(|p| Ok(entries::list(p))).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(
            state.read(|p| entries::url_to_open(p, id, 0)).unwrap().as_str(),
            "https://github.com/"
        );
    }

    #[test]
    fn copy_field_uses_the_clipboard_and_lock_clears_it() {
        use crate::{clipboard::fake::FakeClipboard, entries::EntryInput};
        let tmp = tempfile::tempdir().unwrap();
        let fake = FakeClipboard::default();
        let state = AppState::new(Paths::new(tmp.path()), TEST_KDF, SecureClipboard::new(fake.clone()));
        state.create_vault(PW).unwrap();
        let input = EntryInput {
            id: None,
            title: "GitHub".into(),
            username: "alice".into(),
            password: Some("gh-secret".into()),
            urls: Vec::new(),
            notes: String::new(),
            tags: Vec::new(),
            favorite: false,
        };
        let id = state.edit(|p, ts| entries::save(p, &input, ts)).unwrap();

        state.copy_field(id, FieldRef::Password).unwrap();
        assert_eq!(fake.text().as_deref(), Some("gh-secret"));
        assert_eq!(&*state.reveal_secret(id, FieldRef::Password).unwrap(), "gh-secret");

        state.lock();
        assert_eq!(fake.text(), None, "lock clears our copy");
        assert_eq!(state.copy_field(id, FieldRef::Password).unwrap_err(), AppError::Locked);
        assert_eq!(
            state.reveal_secret(id, FieldRef::Password).unwrap_err(),
            AppError::Locked
        );
    }
}
