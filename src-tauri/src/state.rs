//! App state and the vault lifecycle (create, unlock, lock, change password, new kit).
//!
//! Plain Rust, no Tauri types, so the command handlers can be tested with a temp
//! directory. The Tauri commands in `commands::session` are thin wrappers.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rand::rngs::OsRng;
use serde::Serialize;
use xpass_core::{crypto::KdfParams, model::Payload, Unlock, UnlockedVault};
use zeroize::Zeroizing;

use crate::{
    clipboard::SecureClipboard,
    entries::{self, FieldRef},
    error::{AppError, AppResult},
    lock_watch::LockReason,
    paths::Paths,
    session::VaultSession,
    settings::{self, Settings},
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

/// Status plus why the vault was last locked. The frontend re-reads this when the
/// window becomes visible again, so it does not depend on receiving `vault://locked`
/// (WebView2 can be frozen while Windows sleeps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub status: VaultStatus,
    /// Set only while locked, after a lock in this run of the app.
    pub lock_reason: Option<LockReason>,
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
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    /// Lock after this much system-wide inactivity; `None` = never. From settings,
    /// or `XPASS_IDLE_SECS` in debug builds.
    idle_timeout: Mutex<Option<Duration>>,
    last_lock_reason: Mutex<Option<LockReason>>,
}

impl AppState {
    pub fn new(paths: Paths, target_kdf: KdfParams, clipboard: SecureClipboard) -> Self {
        let settings_path = paths.settings();
        let loaded = settings::load(&settings_path);
        Self(Arc::new(Inner {
            storage: Storage::new(paths),
            target_kdf,
            session: Mutex::new(None),
            throttle: Mutex::new(Throttle::default()),
            gate: Mutex::new(()),
            clipboard,
            settings_path,
            idle_timeout: Mutex::new(loaded.idle_timeout()),
            settings: Mutex::new(loaded),
            last_lock_reason: Mutex::new(None),
        }))
    }

    pub fn settings(&self) -> Settings {
        lock(&self.0.settings).clone()
    }

    /// Validates, saves and applies new settings. `last_kit_at` is kept as is (it is
    /// owned by the app, not the user).
    pub fn update_settings(&self, mut new: Settings) -> AppResult<Settings> {
        new.validate()?;
        let mut current = lock(&self.0.settings);
        new.last_kit_at = current.last_kit_at;
        settings::save(&self.0.settings_path, &new)?;
        self.set_idle_timeout(new.idle_timeout());
        *current = new.clone();
        Ok(new)
    }

    /// Records that a new Emergency Kit was shown. Best effort: the kit date is
    /// informational only.
    fn mark_kit_generated(&self) {
        let mut current = lock(&self.0.settings);
        current.last_kit_at = Some(now_ms());
        if let Err(e) = settings::save(&self.0.settings_path, &current) {
            eprintln!("xpass: saving settings failed: {e}");
        }
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

    pub fn session_info(&self) -> SessionInfo {
        let status = self.status();
        let lock_reason = if status == VaultStatus::Locked {
            *lock(&self.0.last_lock_reason)
        } else {
            None
        };
        SessionInfo { status, lock_reason }
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
        self.mark_kit_generated();
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
        drop(guard);
        self.mark_kit_generated();
        recovery_key.ok_or(AppError::Internal)
    }

    /// Compromise response: new VaultKey, password slot re-created with the same
    /// password, new recovery key (returned for a new Emergency Kit). Always needs the
    /// current password. Older files (backups) still open with the old credentials.
    pub fn rotate_vault_key(&self, current: &str) -> AppResult<Zeroizing<String>> {
        let _gate = lock(&self.0.gate);
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        self.verify_password(session, current)?;
        let target = self.0.target_kdf;
        let recovery_key = session.persist_with(&self.0.storage, now_ms(), |vault| {
            vault
                .rotate(&mut OsRng, current.as_bytes(), target)
                .map_err(AppError::from)
        })?;
        drop(guard);
        self.mark_kit_generated();
        Ok(recovery_key)
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
        self.verify_password(session, current)
    }

    /// Checks the password against the password slot, throttled like unlock.
    fn verify_password(&self, session: &VaultSession, current: &str) -> AppResult<()> {
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

    pub fn idle_timeout(&self) -> Option<Duration> {
        *lock(&self.0.idle_timeout)
    }

    pub fn set_idle_timeout(&self, timeout: Option<Duration>) {
        *lock(&self.0.idle_timeout) = timeout;
    }

    /// Drops the session (wiping keys and secrets) and clears the clipboard if it
    /// still holds a secret we copied. Returns whether the vault was unlocked.
    pub fn lock(&self) -> bool {
        self.0.clipboard.clear_if_ours();
        lock(&self.0.session).take().is_some()
    }

    /// Locks and remembers why, if the vault was unlocked. Returns whether it was.
    pub fn lock_with(&self, reason: LockReason) -> bool {
        let was_unlocked = self.lock();
        if was_unlocked {
            *lock(&self.0.last_lock_reason) = Some(reason);
        }
        was_unlocked
    }

    /// Copies an entry field to the clipboard; it is cleared after the timeout.
    pub fn copy_field(&self, id: uuid::Uuid, field: FieldRef) -> AppResult<()> {
        let value = self.read(|p| entries::field_value(p, id, field))?;
        self.0.clipboard.copy(&value, self.settings().clipboard_clear())
    }

    /// Copies arbitrary secret text (e.g. a generated password) the same secure way.
    pub fn copy_text(&self, text: &str) -> AppResult<()> {
        if text.is_empty() {
            return Err(AppError::InvalidInput { field: "text" });
        }
        self.0.clipboard.copy(text, self.settings().clipboard_clear())
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
        *lock(&self.0.last_lock_reason) = None;
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

    #[test]
    fn session_info_keeps_the_lock_reason_until_unlock() {
        let (_tmp, _paths, state, _) = created();
        let info = state.session_info();
        assert_eq!((info.status, info.lock_reason), (VaultStatus::Unlocked, None));

        assert!(state.lock_with(LockReason::DisplayOff));
        assert!(!state.lock_with(LockReason::Suspend), "already locked: reason kept");
        let info = state.session_info();
        assert_eq!(
            (info.status, info.lock_reason),
            (VaultStatus::Locked, Some(LockReason::DisplayOff))
        );
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"status":"locked","lockReason":"displayOff"}"#
        );

        state.unlock_password(PW).unwrap();
        state.lock();
        assert_eq!(state.session_info().lock_reason, None, "reason cleared by the unlock");
    }

    #[test]
    fn rotate_invalidates_the_old_recovery_key_but_not_old_backups() {
        let (_tmp, paths, state, old_rec) = created();
        assert!(state.settings().last_kit_at.is_some(), "kit date set on create");
        let before = fs::read(paths.vault()).unwrap();

        assert_eq!(state.rotate_vault_key("wrong").unwrap_err(), AppError::BadCredentials);
        let new_rec = state.rotate_vault_key(PW).unwrap();
        assert_ne!(*new_rec, old_rec);

        state.lock();
        assert_eq!(state.unlock_recovery(&old_rec).unwrap_err(), AppError::BadCredentials);
        state.unlock_recovery(&new_rec).unwrap();
        state.lock();
        state.unlock_password(PW).unwrap();

        // A file saved before the rotation (backups, git history) still opens with
        // the old credentials: expected, and explained in the UI.
        let backups = Storage::new(paths.clone()).backups().unwrap();
        assert!(backups.iter().any(|b| fs::read(b).unwrap() == before));
        UnlockedVault::unlock(&before, Unlock::Recovery(&old_rec)).unwrap();
        UnlockedVault::unlock(&before, Unlock::Password(PW.as_bytes())).unwrap();
    }

    #[test]
    fn settings_are_saved_applied_and_reloaded() {
        let (_tmp, paths, state, _) = created();
        let kit_at = state.settings().last_kit_at;
        let mut s = state.settings();
        s.auto_lock_minutes = Some(15);
        s.clipboard_clear_seconds = 45;
        s.last_kit_at = Some(1); // ignored: owned by the app
        let saved = state.update_settings(s).unwrap();
        assert_eq!(saved.last_kit_at, kit_at);
        assert_eq!(state.idle_timeout(), Some(Duration::from_secs(15 * 60)));

        let mut bad = state.settings();
        bad.auto_lock_minutes = Some(7);
        assert!(matches!(
            state.update_settings(bad).unwrap_err(),
            AppError::InvalidInput { .. }
        ));

        let restarted = AppState::new(paths, TEST_KDF, test_clipboard());
        assert_eq!(restarted.settings().clipboard_clear_seconds, 45);
        assert_eq!(restarted.idle_timeout(), Some(Duration::from_secs(15 * 60)));
    }
}
