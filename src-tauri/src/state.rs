//! App state and the vault lifecycle (create, unlock, lock, change password, new kit,
//! Windows Hello).
//!
//! Plain Rust, no Tauri types, so the command handlers can be tested with a temp
//! directory. The Tauri commands in `commands::session` are thin wrappers.

mod backups;
mod export;
mod import;
mod reauth;
mod sync;

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rand::rngs::OsRng;
use serde::Serialize;
use uuid::Uuid;
use xpass_core::{
    crypto::KdfParams,
    format::{self, SlotKind},
    model::Payload,
    vault::{hello_challenge, hello_challenge_for_file},
    Unlock, UnlockedVault,
};
use zeroize::Zeroizing;

use crate::{
    clipboard::SecureClipboard,
    entries::{self, FieldRef},
    error::{AppError, AppResult},
    github::{ConnectionReport, GitHubStore},
    hello::{self, HelloBackend, HelloError},
    lock_watch::LockReason,
    paths::Paths,
    secrets::{self, MemoryTokens, TokenStore},
    session::VaultSession,
    settings::{self, Settings, SyncSettings},
    storage::Storage,
    strength, sync_state,
    sync_worker::Trigger,
    throttle::Throttle,
};

pub use backups::BackupInfo;
pub use export::{suggested_file_name, ExportFormat, ExportResult};
pub use import::ImportPreview;
pub use reauth::Auth;
pub use sync::{SyncPhase, SyncStatus};

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
    pub hello: HelloInfo,
}

/// Windows Hello state of the vault, readable while locked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloInfo {
    /// The vault (file on disk, or the open session) has a Hello slot.
    pub enrolled: bool,
    /// A Hello unlock found the credential missing or changed in this run of the
    /// app. The slot is removed at the next password or recovery unlock.
    pub broken: bool,
    /// The Hello slot was dropped (broken credential or VaultKey rotation); the UI
    /// offers to set Hello up again until the user acts on it or dismisses it.
    pub reenroll_suggested: bool,
}

/// [`HelloInfo`] plus whether Windows Hello is usable on this PC (that check may
/// take a moment, so it is a separate command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloStatus {
    pub supported: bool,
    #[serde(flatten)]
    pub info: HelloInfo,
}

#[derive(Debug, Default)]
struct HelloFlags {
    broken: bool,
    reenroll_suggested: bool,
}

/// How a vault was unlocked.
#[derive(Clone, Copy)]
enum Method<'a> {
    Password(&'a str),
    Recovery,
    Hello,
}

/// Sync setup as shown in Settings. The token itself never leaves Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    pub repo: String,
    pub branch: String,
    pub path: String,
    pub enabled: bool,
    /// A token is stored in Windows Credential Manager.
    pub has_token: bool,
}

/// Result of a VaultKey rotation.
pub struct Rotated {
    /// New recovery key for the Emergency Kit. Shown once, never stored.
    pub recovery_key: Zeroizing<String>,
    /// Windows Hello was on and is now off (the UI offers to set it up again).
    pub hello_dropped: bool,
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
    hello: Box<dyn HelloBackend>,
    hello_flags: Mutex<HelloFlags>,
    /// Held while a Hello dialog is open, so a second request fails fast instead
    /// of queueing another dialog.
    hello_busy: Mutex<()>,
    /// GitHub token (Windows Credential Manager in the app).
    tokens: Box<dyn TokenStore>,
    sync_state_path: PathBuf,
    /// Bumped (under the session lock) on every local change of the vault file and
    /// on unlock; a sync commit compares it with its snapshot (S2).
    changes: AtomicU64,
    /// Bumped on every local password / recovery slot change (S5).
    slot_changes: AtomicU64,
    sync_status: Mutex<SyncStatus>,
    sync_notifier: OnceLock<Box<dyn Fn(Trigger) + Send + Sync>>,
    /// Held by everything that talks to the remote (sync, key-less push, join,
    /// key-change resolution, replace, online rotation): one at a time.
    sync_running: tokio::sync::Mutex<()>,
    /// Pending CSV import (v0.5 I3). Lock order: `session`, then `import`.
    import: Mutex<import::ImportSlot>,
}

impl AppState {
    pub fn new(paths: Paths, target_kdf: KdfParams, clipboard: SecureClipboard, hello: Box<dyn HelloBackend>) -> Self {
        let settings_path = paths.settings();
        let sync_state_path = paths.sync_state();
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
            hello,
            hello_flags: Mutex::new(HelloFlags::default()),
            hello_busy: Mutex::new(()),
            tokens: Box::new(MemoryTokens::default()),
            sync_state_path,
            changes: AtomicU64::new(0),
            slot_changes: AtomicU64::new(0),
            sync_status: Mutex::new(SyncStatus::default()),
            sync_notifier: OnceLock::new(),
            sync_running: tokio::sync::Mutex::new(()),
            import: Mutex::new(import::ImportSlot::default()),
        }))
    }

    /// Installs the token store (the app: Windows Credential Manager). Must be
    /// called before the state is shared.
    pub fn with_token_store(mut self, tokens: Box<dyn TokenStore>) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("token store is set before the state is shared")
            .tokens = tokens;
        self
    }

    // ---- sync setup (T19) ----

    pub fn sync_config(&self) -> AppResult<SyncConfig> {
        let sync = self.settings().sync;
        Ok(SyncConfig {
            has_token: self.0.tokens.get()?.is_some(),
            repo: sync.repo,
            branch: sync.branch,
            path: sync.path,
            enabled: sync.enabled,
        })
    }

    /// Changing where the vault syncs is refused while the vault is locked, so a
    /// walk-up user cannot point a locked app at another repository.
    fn ensure_not_locked(&self) -> AppResult<()> {
        match self.status() {
            VaultStatus::Locked => Err(AppError::Locked),
            VaultStatus::Unlocked | VaultStatus::NoVault => Ok(()),
        }
    }

    /// Saves settings with a new sync block.
    fn save_sync_settings(&self, current: &mut Settings, sync: SyncSettings) -> AppResult<()> {
        let mut next = current.clone();
        next.sync = sync;
        settings::save(&self.0.settings_path, &next)?;
        *current = next;
        Ok(())
    }

    /// Saves a new sync target (already normalized by `SyncSettings::from_input`).
    /// A different repository, branch or path resets the sync state.
    pub fn configure_sync(&self, mut new: SyncSettings) -> AppResult<SyncConfig> {
        self.ensure_not_locked()?;
        new.enabled = self.0.tokens.get()?.is_some();
        new.validate()?;
        {
            let mut current = lock(&self.0.settings);
            let changed = current.sync.remote_id() != new.remote_id();
            self.save_sync_settings(&mut current, new)?;
            if changed {
                sync_state::reset(&self.0.sync_state_path)?;
            }
        }
        self.notify(Trigger::ConfigChanged);
        self.sync_config()
    }

    /// Stores the GitHub token (validated) and turns sync on if a repository is set.
    pub fn set_sync_token(&self, token: &str) -> AppResult<SyncConfig> {
        self.ensure_not_locked()?;
        let token = secrets::normalize_token(token)?;
        self.0.tokens.set(&token)?;
        {
            let mut current = lock(&self.0.settings);
            if current.sync.is_configured() && !current.sync.enabled {
                let sync = SyncSettings {
                    enabled: true,
                    ..current.sync.clone()
                };
                self.save_sync_settings(&mut current, sync)?;
            }
        }
        self.notify(Trigger::ConfigChanged);
        self.sync_config()
    }

    /// Turns sync off: deletes the token, the sync target and the sync state.
    /// The local vault is not touched.
    pub fn disconnect_sync(&self) -> AppResult<SyncConfig> {
        self.ensure_not_locked()?;
        self.0.tokens.delete()?;
        {
            let mut current = lock(&self.0.settings);
            self.save_sync_settings(&mut current, SyncSettings::default())?;
        }
        sync_state::reset(&self.0.sync_state_path)?;
        *lock(&self.0.sync_status) = SyncStatus::default();
        self.notify(Trigger::ConfigChanged);
        self.sync_config()
    }

    /// Store for a sync target with either the given token (typed in the form, not
    /// saved yet) or the stored one.
    pub fn github_store(&self, target: &SyncSettings, token: Option<&str>) -> AppResult<GitHubStore> {
        let token = match token {
            Some(t) => secrets::normalize_token(t)?,
            None => self.0.tokens.get()?.ok_or(AppError::SyncNotConfigured)?,
        };
        Ok(GitHubStore::new(token, target)?)
    }

    /// Id of this PC's vault, read from the file header (works while locked).
    pub fn local_vault_id(&self) -> Option<Uuid> {
        let bytes = self.0.storage.read().ok()??;
        let parsed = format::parse(&bytes).ok()?;
        Some(Uuid::from_bytes(parsed.header.vault_id))
    }

    /// "Test connection" for the values in the form (S12), before saving them.
    pub async fn test_sync_connection(
        &self,
        target: SyncSettings,
        token: Option<Zeroizing<String>>,
    ) -> AppResult<ConnectionReport> {
        self.ensure_not_locked()?;
        let store = self.github_store(&target, token.as_deref().map(String::as_str))?;
        Ok(store.test_connection(self.local_vault_id()).await?)
    }

    pub fn settings(&self) -> Settings {
        lock(&self.0.settings).clone()
    }

    /// Validates, saves and applies new settings. `last_kit_at` is kept as is (it is
    /// owned by the app, not the user).
    /// The sync block is owned by the sync commands and also kept.
    pub fn update_settings(&self, mut new: Settings) -> AppResult<Settings> {
        let mut current = lock(&self.0.settings);
        new.last_kit_at = current.last_kit_at;
        new.last_update_check_at = current.last_update_check_at;
        new.sync = current.sync.clone();
        new.validate()?;
        settings::save(&self.0.settings_path, &new)?;
        self.set_idle_timeout(new.idle_timeout());
        *current = new.clone();
        Ok(new)
    }

    /// Records when an update check ran (startup throttle, v0.6 H9). Best effort.
    pub fn record_update_check(&self, at: i64) {
        let mut current = lock(&self.0.settings);
        current.last_update_check_at = Some(at);
        if let Err(e) = settings::save(&self.0.settings_path, &current) {
            eprintln!("xpass: saving settings failed: {e}");
        }
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
        SessionInfo {
            status,
            lock_reason,
            hello: self.hello_info(),
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
        self.mark_kit_generated();
        Ok(recovery_key)
    }

    pub fn unlock_password(&self, password: &str) -> AppResult<()> {
        let _gate = lock(&self.0.gate);
        self.unlock(None, Unlock::Password(password.as_bytes()), Method::Password(password))
    }

    pub fn unlock_recovery(&self, recovery_key: &str) -> AppResult<()> {
        let _gate = lock(&self.0.gate);
        self.unlock(None, Unlock::Recovery(recovery_key), Method::Recovery)
    }

    /// Unlocks with Windows Hello. Blocks until the user answers the Hello dialog;
    /// no lock is held meanwhile, so a password unlock can still finish first (the
    /// Hello result is then discarded).
    pub fn unlock_hello(&self) -> AppResult<()> {
        if lock(&self.0.session).is_some() {
            return Ok(());
        }
        if lock(&self.0.hello_flags).broken {
            return Err(AppError::HelloNotFound);
        }
        let _busy = self.0.hello_busy.try_lock().map_err(|_| AppError::HelloBusy)?;
        let file = self.0.storage.read()?.ok_or(AppError::NoVault)?;
        let challenge = hello_challenge_for_file(&file)?;
        let vault_id = Uuid::from_bytes(format::parse(&file)?.header.vault_id);
        let signature = match self.0.hello.sign(&hello::credential_name(vault_id), &challenge) {
            Ok(signature) => signature,
            Err(HelloError::NotFound) => {
                lock(&self.0.hello_flags).broken = true;
                return Err(AppError::HelloNotFound);
            }
            Err(e) => return Err(e.into()),
        };

        let _gate = lock(&self.0.gate);
        // Unlock the same file the challenge came from, so a concurrent change of
        // the Hello slot cannot be mistaken for a changed credential.
        match self.unlock(Some(&file), Unlock::Hello { signature: &signature }, Method::Hello) {
            // The signature does not open the slot: the credential was re-created.
            Err(AppError::BadCredentials) => {
                lock(&self.0.hello_flags).broken = true;
                Err(AppError::HelloNotFound)
            }
            other => other,
        }
    }

    /// Windows Hello state without asking Windows (fast; part of `session_info`).
    pub fn hello_info(&self) -> HelloInfo {
        let unlocked = lock(&self.0.session)
            .as_ref()
            .map(|session| session.vault().slot_kinds().contains(&SlotKind::Hello));
        let enrolled = unlocked.unwrap_or_else(
            || matches!(self.0.storage.read(), Ok(Some(file)) if hello_challenge_for_file(&file).is_ok()),
        );
        let flags = lock(&self.0.hello_flags);
        HelloInfo {
            enrolled,
            broken: flags.broken,
            reenroll_suggested: flags.reenroll_suggested,
        }
    }

    /// [`Self::hello_info`] plus whether Hello is usable on this PC (may block briefly).
    pub fn hello_status(&self) -> HelloStatus {
        HelloStatus {
            supported: self.0.hello.is_supported(),
            info: self.hello_info(),
        }
    }

    /// Turns Hello on: creates the vault's Hello credential if needed and adds a
    /// Hello slot. Needs the current password unless the session was unlocked with
    /// the recovery key. Blocks on the Hello dialog without holding the session lock.
    pub fn enable_hello(&self, current: Option<&str>) -> AppResult<()> {
        let _busy = self.0.hello_busy.try_lock().map_err(|_| AppError::HelloBusy)?;
        let vault_id = {
            let guard = lock(&self.0.session);
            let session = guard.as_ref().ok_or(AppError::Locked)?;
            self.authorize(session, current)?;
            session.vault().vault_id()
        };
        if !self.0.hello.is_supported() {
            return Err(AppError::HelloUnavailable);
        }
        let name = hello::credential_name(vault_id);
        self.0.hello.open_or_create(&name)?;
        let salt = UnlockedVault::new_hello_salt(&mut OsRng);
        let signature = self.0.hello.sign(&name, &hello_challenge(vault_id, &salt))?;

        let mut guard = lock(&self.0.session);
        // Locked (or a different vault opened) while the dialog was open.
        let session = guard
            .as_mut()
            .filter(|s| s.vault().vault_id() == vault_id)
            .ok_or(AppError::Locked)?;
        session.persist_with(&self.0.storage, now_ms(), |vault| {
            vault.set_hello_slot(&mut OsRng, salt, &signature);
            Ok(())
        })?;
        self.changed(false);
        drop(guard);
        *lock(&self.0.hello_flags) = HelloFlags::default();
        Ok(())
    }

    /// Turns Hello off: removes the Hello slot and deletes the credential.
    pub fn disable_hello(&self) -> AppResult<()> {
        let vault_id = {
            let mut guard = lock(&self.0.session);
            let session = guard.as_mut().ok_or(AppError::Locked)?;
            if session.vault().slot_kinds().contains(&SlotKind::Hello) {
                session.persist_with(&self.0.storage, now_ms(), |vault| {
                    vault.remove_hello_slot();
                    Ok(())
                })?;
                self.changed(false);
            }
            session.vault().vault_id()
        };
        *lock(&self.0.hello_flags) = HelloFlags::default();
        self.delete_hello_credential(vault_id);
        Ok(())
    }

    /// Hides the "set up Windows Hello again" suggestion.
    pub fn dismiss_hello_reenroll(&self) {
        lock(&self.0.hello_flags).reenroll_suggested = false;
    }

    /// Best effort: a leftover credential only holds a key that wraps nothing.
    fn delete_hello_credential(&self, vault_id: Uuid) {
        if self.0.hello.delete(&hello::credential_name(vault_id)).is_err() {
            eprintln!("xpass: deleting the Windows Hello credential failed");
        }
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
        })?;
        self.changed(true);
        Ok(())
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
        self.changed(true);
        drop(guard);
        self.mark_kit_generated();
        recovery_key.ok_or(AppError::Internal)
    }

    /// Compromise response: new VaultKey, password slot re-created with the same
    /// password, new recovery key (returned for a new Emergency Kit). Always needs the
    /// current password. Older files (backups) still open with the old credentials.
    ///
    /// The Hello slot is dropped too (re-enrolling needs a new Hello signature): the
    /// result says whether it was, and the old credential is deleted.
    pub fn rotate_vault_key(&self, current: &str) -> AppResult<Rotated> {
        let _gate = lock(&self.0.gate);
        let mut guard = lock(&self.0.session);
        let session = guard.as_mut().ok_or(AppError::Locked)?;
        self.verify_password(session, current)?;
        let target = self.0.target_kdf;
        let hello_dropped = session.vault().slot_kinds().contains(&SlotKind::Hello);
        let vault_id = session.vault().vault_id();
        let recovery_key = session.persist_with(&self.0.storage, now_ms(), |vault| {
            vault
                .rotate(&mut OsRng, current.as_bytes(), target)
                .map_err(AppError::from)
        })?;
        self.changed(true);
        drop(guard);
        self.mark_kit_generated();
        if hello_dropped {
            lock(&self.0.hello_flags).reenroll_suggested = true;
            self.delete_hello_credential(vault_id);
        }
        Ok(Rotated {
            recovery_key,
            hello_dropped,
        })
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
        let out = session.apply(&self.0.storage, now_ms(), f)?;
        self.changed(false);
        Ok(out)
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
        let was_unlocked = lock(&self.0.session).take().is_some();
        self.clear_import();
        was_unlocked
    }

    /// Locks and remembers why, if the vault was unlocked. Returns whether it was.
    /// Except on app exit and before an update install (both push on their own,
    /// S8), the sync worker is told so it can push the local file without the key.
    pub fn lock_with(&self, reason: LockReason) -> bool {
        let was_unlocked = self.lock();
        if was_unlocked {
            *lock(&self.0.last_lock_reason) = Some(reason);
            if !matches!(reason, LockReason::Exit | LockReason::Update) {
                self.notify(Trigger::Locked);
            }
        }
        was_unlocked
    }

    /// Copies an entry field to the clipboard; it is cleared after the timeout.
    pub fn copy_field(&self, id: uuid::Uuid, field: FieldRef) -> AppResult<()> {
        let value = self.read(|p| entries::field_value(p, id, field, now_ms()))?;
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

    /// Opens the vault (`file`, or the one on disk) and starts the session. The
    /// caller holds the gate.
    ///
    /// Hello unlocks skip the throttle: each attempt needs user verification by
    /// Windows (which has its own lockout), and a failure is not a password guess.
    fn unlock(&self, file: Option<&[u8]>, how: Unlock<'_>, method: Method<'_>) -> AppResult<()> {
        if lock(&self.0.session).is_some() {
            return Ok(());
        }
        let hello = matches!(method, Method::Hello);
        if !hello {
            self.check_throttle()?;
        }
        let read;
        let file = match file {
            Some(file) => file,
            None => {
                read = self.0.storage.read()?.ok_or(AppError::NoVault)?;
                &read
            }
        };

        let vault = match UnlockedVault::unlock(file, how) {
            Ok(vault) => vault,
            Err(xpass_core::Error::BadCredentials) => {
                if !hello {
                    lock(&self.0.throttle).record_failure(Instant::now());
                }
                return Err(AppError::BadCredentials);
            }
            // A recovery-key typo is caught by its checksum and is not a guess.
            Err(e) => return Err(e.into()),
        };
        if !hello {
            lock(&self.0.throttle).record_success();
        }

        let mut session = VaultSession::new(vault, matches!(method, Method::Recovery));
        if let Method::Password(password) = method {
            self.rewrap_if_weak(&mut session, password);
        }
        if hello {
            lock(&self.0.hello_flags).broken = false;
        } else {
            self.drop_broken_hello_slot(&mut session);
        }
        self.start_session(session);
        Ok(())
    }

    /// Makes `session` the open vault and tells the sync worker.
    fn start_session(&self, session: VaultSession) {
        {
            let mut guard = lock(&self.0.session);
            *guard = Some(session);
            self.0.changes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *lock(&self.0.last_lock_reason) = None;
        self.notify(Trigger::Unlocked);
    }

    /// After a Hello unlock found the credential missing or changed, the next
    /// password or recovery unlock removes the dead Hello slot and suggests setting
    /// Hello up again. Best effort: if saving fails, the flag stays and the next
    /// unlock retries.
    fn drop_broken_hello_slot(&self, session: &mut VaultSession) {
        let mut flags = lock(&self.0.hello_flags);
        if !flags.broken {
            return;
        }
        if session.vault().slot_kinds().contains(&SlotKind::Hello) {
            let result = session.persist_with(&self.0.storage, now_ms(), |vault| {
                vault.remove_hello_slot();
                Ok(())
            });
            if let Err(e) = result {
                eprintln!("xpass: removing the Windows Hello slot failed: {e}");
                return;
            }
        }
        flags.broken = false;
        flags.reenroll_suggested = true;
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
    use crate::hello::fake::FakeHello;

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
        let (tmp, paths, state, _) = setup_hello();
        (tmp, paths, state)
    }

    fn setup_hello() -> (tempfile::TempDir, Paths, AppState, FakeHello) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("data"));
        let fake = FakeHello::default();
        let state = AppState::new(paths.clone(), TEST_KDF, test_clipboard(), Box::new(fake.clone()));
        (tmp, paths, state, fake)
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

    // ---- sync setup (T19) ----

    const TOKEN: &str = "github_pat_0123456789abcdefghij";

    #[test]
    fn sync_setup_token_and_disconnect() {
        let (_tmp, paths, state) = setup();
        let cfg = state.sync_config().unwrap();
        assert!(!cfg.enabled && !cfg.has_token && cfg.repo.is_empty());

        let target = SyncSettings::from_input("octo/vault", "", "").unwrap();
        let cfg = state.configure_sync(target).unwrap();
        assert_eq!(cfg.repo, "octo/vault");
        assert!(!cfg.enabled, "no token yet");

        let cfg = state.set_sync_token(&format!("  {TOKEN}\n")).unwrap();
        assert!(cfg.enabled && cfg.has_token);

        // The token is in the token store only: not in settings.json.
        let on_disk = fs::read_to_string(paths.settings()).unwrap();
        assert!(!on_disk.contains("github_pat_"), "{on_disk}");
        assert!(on_disk.contains("octo/vault"));

        let cfg = state.disconnect_sync().unwrap();
        assert!(!cfg.enabled && !cfg.has_token && cfg.repo.is_empty());
    }

    #[test]
    fn invalid_token_is_rejected_without_echoing_it() {
        let (_tmp, _paths, state) = setup();
        let err = state.set_sync_token("not a token").unwrap_err();
        assert_eq!(err, AppError::InvalidInput { field: "token" });
        assert!(!serde_json::to_string(&err).unwrap().contains("not a token"));
        assert!(!state.sync_config().unwrap().has_token);
    }

    #[test]
    fn sync_setup_is_refused_while_locked() {
        let (_tmp, _paths, state, _) = created();
        state.lock();
        let target = SyncSettings::from_input("octo/vault", "", "").unwrap();
        assert_eq!(state.configure_sync(target).unwrap_err(), AppError::Locked);
        assert_eq!(state.set_sync_token(TOKEN).unwrap_err(), AppError::Locked);
        assert_eq!(state.disconnect_sync().unwrap_err(), AppError::Locked);
    }

    #[test]
    fn changing_the_remote_resets_the_sync_state() {
        let (_tmp, paths, state) = setup();
        state
            .configure_sync(SyncSettings::from_input("octo/vault", "", "").unwrap())
            .unwrap();
        fs::write(paths.sync_state(), b"{}").unwrap();
        state
            .configure_sync(SyncSettings::from_input("Octo/Vault", "", "").unwrap())
            .unwrap();
        assert!(paths.sync_state().exists(), "same remote (owner case only): kept");
        state
            .configure_sync(SyncSettings::from_input("octo/vault", "", "other.xpv").unwrap())
            .unwrap();
        assert!(!paths.sync_state().exists(), "new path: reset");
    }

    #[test]
    fn update_settings_keeps_the_sync_block() {
        let (_tmp, _paths, state) = setup();
        state
            .configure_sync(SyncSettings::from_input("octo/vault", "", "").unwrap())
            .unwrap();
        let mut from_ui = state.settings();
        from_ui.sync = SyncSettings::default();
        from_ui.clipboard_clear_seconds = 30;
        let saved = state.update_settings(from_ui).unwrap();
        assert_eq!(saved.sync.repo, "octo/vault");
        assert_eq!(saved.clipboard_clear_seconds, 30);
    }

    #[test]
    fn local_vault_id_reads_the_header_while_locked() {
        let (_tmp, _paths, state) = setup();
        assert_eq!(state.local_vault_id(), None);
        state.create_vault(PW).unwrap();
        let id = state.local_vault_id().unwrap();
        state.lock();
        assert_eq!(state.local_vault_id(), Some(id));
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
        let restarted = AppState::new(paths, TEST_KDF, test_clipboard(), Box::new(FakeHello::default()));
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
        let upgraded = AppState::new(
            paths.clone(),
            stronger,
            test_clipboard(),
            Box::new(FakeHello::default()),
        );

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
            fields: None,
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
        let state = AppState::new(
            Paths::new(tmp.path()),
            TEST_KDF,
            SecureClipboard::new(fake.clone()),
            Box::new(FakeHello::default()),
        );
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
            fields: None,
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
    fn copying_a_totp_puts_the_current_code_on_the_clipboard() {
        use crate::{
            clipboard::fake::FakeClipboard,
            entries::{EntryInput, FieldInput, FieldValue},
        };
        let tmp = tempfile::tempdir().unwrap();
        let fake = FakeClipboard::default();
        let state = AppState::new(
            Paths::new(tmp.path()),
            TEST_KDF,
            SecureClipboard::new(fake.clone()),
            Box::new(FakeHello::default()),
        );
        state.create_vault(PW).unwrap();
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let input = EntryInput {
            id: None,
            title: "Mail".into(),
            username: String::new(),
            password: None,
            urls: Vec::new(),
            notes: String::new(),
            tags: Vec::new(),
            favorite: false,
            fields: Some(vec![FieldInput {
                name: "TOTP".into(),
                secret: true,
                value: FieldValue::Set(secret.into()),
            }]),
        };
        let id = state.edit(|p, ts| entries::save(p, &input, ts)).unwrap();

        state.copy_field(id, FieldRef::Totp { index: 0 }).unwrap();
        let copied = fake.text().unwrap();
        assert!(
            copied.len() == 6 && copied.bytes().all(|b| b.is_ascii_digit()),
            "{copied}"
        );
        let shown = state.read(|p| entries::totp_code(p, id, 0, now_ms())).unwrap();
        assert_eq!(shown.period, 30);

        state.lock();
        assert_eq!(fake.text(), None);
        assert_eq!(
            state.read(|p| entries::totp_code(p, id, 0, now_ms())).unwrap_err(),
            AppError::Locked
        );
        assert_eq!(
            state.copy_field(id, FieldRef::Totp { index: 0 }).unwrap_err(),
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
            r#"{"status":"locked","lockReason":"displayOff","hello":{"enrolled":false,"broken":false,"reenrollSuggested":false}}"#
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

        assert_eq!(state.rotate_vault_key("wrong").err(), Some(AppError::BadCredentials));
        let rotated = state.rotate_vault_key(PW).unwrap();
        assert!(!rotated.hello_dropped);
        let new_rec = rotated.recovery_key;
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
        s.last_update_check_at = Some(2); // ignored: owned by the app
        let saved = state.update_settings(s).unwrap();
        assert_eq!(saved.last_kit_at, kit_at);
        assert_eq!(saved.last_update_check_at, None);
        assert!(
            !saved.check_updates_on_startup,
            "startup update check is off by default"
        );
        state.record_update_check(42);
        assert_eq!(settings::load(&paths.settings()).last_update_check_at, Some(42));
        assert_eq!(state.idle_timeout(), Some(Duration::from_secs(15 * 60)));

        let mut bad = state.settings();
        bad.auto_lock_minutes = Some(7);
        assert!(matches!(
            state.update_settings(bad).unwrap_err(),
            AppError::InvalidInput { .. }
        ));

        let restarted = AppState::new(paths, TEST_KDF, test_clipboard(), Box::new(FakeHello::default()));
        assert_eq!(restarted.settings().clipboard_clear_seconds, 45);
        assert_eq!(restarted.idle_timeout(), Some(Duration::from_secs(15 * 60)));
    }

    // --- Windows Hello ---------------------------------------------------------

    /// Created vault (unlocked) with Hello turned on.
    fn hello_enrolled() -> (tempfile::TempDir, Paths, AppState, FakeHello, String) {
        let (tmp, paths, state, fake) = setup_hello();
        let rec = state.create_vault(PW).unwrap().to_string();
        state.enable_hello(Some(PW)).unwrap();
        (tmp, paths, state, fake, rec)
    }

    fn hello_name(state: &AppState) -> String {
        let file = state.0.storage.read().unwrap().unwrap();
        hello::credential_name(Uuid::from_bytes(format::parse(&file).unwrap().header.vault_id))
    }

    fn slots_on_disk(paths: &Paths) -> Vec<SlotKind> {
        let file = fs::read(paths.vault()).unwrap();
        let parsed = format::parse(&file).unwrap();
        parsed.header.slots.iter().map(|s| s.params.kind()).collect()
    }

    #[test]
    fn hello_enable_then_unlock() {
        let (_tmp, paths, state, fake, _) = hello_enrolled();
        assert!(slots_on_disk(&paths).contains(&SlotKind::Hello));
        assert!(fake.has(&hello_name(&state)));
        assert!(state.hello_info().enrolled);

        state.lock();
        let info = state.session_info();
        assert_eq!(info.status, VaultStatus::Locked);
        assert!(info.hello.enrolled, "readable while locked");
        state.unlock_hello().unwrap();
        assert_eq!(state.status(), VaultStatus::Unlocked);
        // Already unlocked: no second prompt.
        let prompts = fake.prompts();
        state.unlock_hello().unwrap();
        assert_eq!(fake.prompts(), prompts);
    }

    #[test]
    fn hello_unlock_is_not_a_recovery_session() {
        let (_tmp, _paths, state, _fake, _) = hello_enrolled();
        state.lock();
        state.unlock_hello().unwrap();
        // Sensitive actions still need the password.
        assert_eq!(
            state.change_password(None, NEW_PW).unwrap_err(),
            AppError::InvalidInput {
                field: "currentPassword"
            }
        );
    }

    #[test]
    fn hello_enable_needs_the_password_except_after_recovery() {
        let (_tmp, paths, state, _fake) = setup_hello();
        let rec = state.create_vault(PW).unwrap().to_string();
        assert_eq!(
            state.enable_hello(None).unwrap_err(),
            AppError::InvalidInput {
                field: "currentPassword"
            }
        );
        assert_eq!(state.enable_hello(Some("wrong")).unwrap_err(), AppError::BadCredentials);
        assert!(!slots_on_disk(&paths).contains(&SlotKind::Hello));

        state.lock();
        state.unlock_recovery(&rec).unwrap();
        state.enable_hello(None).unwrap();
        assert!(slots_on_disk(&paths).contains(&SlotKind::Hello));
    }

    #[test]
    fn hello_enable_failures_change_nothing() {
        let (_tmp, paths, state, fake) = setup_hello();
        state.create_vault(PW).unwrap();
        let before = fs::read(paths.vault()).unwrap();

        fake.fail_next(HelloError::Canceled);
        assert_eq!(state.enable_hello(Some(PW)).unwrap_err(), AppError::HelloCanceled);
        fake.set_supported(false);
        assert_eq!(state.enable_hello(Some(PW)).unwrap_err(), AppError::HelloUnavailable);
        assert!(!state.hello_status().supported);
        assert_eq!(fs::read(paths.vault()).unwrap(), before);

        state.lock();
        assert_eq!(state.enable_hello(Some(PW)).unwrap_err(), AppError::Locked);
    }

    #[test]
    fn hello_unlock_without_slot_or_vault() {
        let (_tmp, _paths, state) = setup();
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::NoVault);
        state.create_vault(PW).unwrap();
        state.lock();
        assert!(!state.hello_info().enrolled);
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::NoSuchSlot);
    }

    #[test]
    fn hello_disable_removes_slot_and_credential() {
        let (_tmp, paths, state, fake, _) = hello_enrolled();
        let name = hello_name(&state);
        state.disable_hello().unwrap();
        assert!(!slots_on_disk(&paths).contains(&SlotKind::Hello));
        assert!(!fake.has(&name));
        state.lock();
        assert!(!state.hello_info().enrolled);
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::NoSuchSlot);
    }

    #[test]
    fn hello_reset_falls_back_to_password_and_drops_the_slot() {
        let (_tmp, paths, state, fake, _) = hello_enrolled();
        state.lock();
        fake.reset();
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::HelloNotFound);
        assert!(state.session_info().hello.broken);
        // No further prompts once broken.
        let prompts = fake.prompts();
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::HelloNotFound);
        assert_eq!(fake.prompts(), prompts);
        assert!(
            slots_on_disk(&paths).contains(&SlotKind::Hello),
            "file unchanged while locked"
        );

        state.unlock_password(PW).unwrap();
        assert!(!slots_on_disk(&paths).contains(&SlotKind::Hello));
        let info = state.hello_info();
        assert!(!info.enrolled && !info.broken && info.reenroll_suggested);

        // Re-enroll works and clears the suggestion.
        state.enable_hello(Some(PW)).unwrap();
        assert!(!state.hello_info().reenroll_suggested);
        state.lock();
        state.unlock_hello().unwrap();
    }

    #[test]
    fn hello_recreated_credential_is_handled_like_a_reset_and_not_throttled() {
        let (_tmp, paths, state, fake, rec) = hello_enrolled();
        state.lock();
        fake.recreate(&hello_name(&state));
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::HelloNotFound);
        assert!(state.hello_info().broken);
        // Not counted as a failed guess: the password unlocks at once.
        state.unlock_password(PW).unwrap();
        assert!(!slots_on_disk(&paths).contains(&SlotKind::Hello));
        assert!(state.hello_info().reenroll_suggested);
        state.dismiss_hello_reenroll();
        assert!(!state.hello_info().reenroll_suggested);
        // The recovery key still works too.
        state.lock();
        state.unlock_recovery(&rec).unwrap();
    }

    #[test]
    fn hello_cancel_and_lockout_keep_the_slot() {
        let (_tmp, paths, state, fake, _) = hello_enrolled();
        state.lock();
        fake.fail_next(HelloError::Canceled);
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::HelloCanceled);
        fake.fail_next(HelloError::DeviceLocked);
        assert_eq!(state.unlock_hello().unwrap_err(), AppError::HelloDeviceLocked);
        assert!(!state.session_info().hello.broken);
        state.unlock_hello().unwrap();
        assert!(slots_on_disk(&paths).contains(&SlotKind::Hello));
    }

    #[test]
    fn hello_unlock_works_while_password_is_throttled() {
        let (_tmp, _paths, state, _fake, _) = hello_enrolled();
        state.lock();
        // Two failures start the delay (0 s after the first, 1 s after the second).
        for _ in 0..2 {
            let _ = state.unlock_password("wrong");
        }
        assert!(matches!(
            state.unlock_password(PW).unwrap_err(),
            AppError::Throttled { .. }
        ));
        state.unlock_hello().unwrap();
    }

    #[test]
    fn hello_survives_password_change_and_new_kit_but_not_rotation() {
        let (_tmp, paths, state, fake, _) = hello_enrolled();
        let name = hello_name(&state);
        state.change_password(Some(PW), NEW_PW).unwrap();
        state.new_recovery_key(Some(NEW_PW)).unwrap();
        state.lock();
        state.unlock_hello().unwrap();

        let rotated = state.rotate_vault_key(NEW_PW).unwrap();
        assert!(rotated.hello_dropped);
        assert!(!slots_on_disk(&paths).contains(&SlotKind::Hello));
        assert!(!fake.has(&name), "old credential deleted");
        assert!(state.hello_info().reenroll_suggested);

        // Re-enroll after rotation uses the same credential name (same vault id).
        state.enable_hello(Some(NEW_PW)).unwrap();
        assert_eq!(fake.names(), vec![name]);
        state.lock();
        state.unlock_hello().unwrap();
    }

    #[test]
    fn hello_credentials_are_per_vault() {
        let (_tmp1, _p1, a, fake) = setup_hello();
        let tmp2 = tempfile::tempdir().unwrap();
        let b = AppState::new(
            Paths::new(tmp2.path().join("data")),
            TEST_KDF,
            test_clipboard(),
            Box::new(fake.clone()),
        );
        for state in [&a, &b] {
            state.create_vault(PW).unwrap();
            state.enable_hello(Some(PW)).unwrap();
        }
        assert_eq!(fake.names().len(), 2);
        a.disable_hello().unwrap();
        b.lock();
        b.unlock_hello().unwrap();
    }

    #[test]
    fn session_info_serializes_hello_state() {
        let (_tmp, _paths, state, _fake, _) = hello_enrolled();
        state.lock();
        let json = serde_json::to_value(state.session_info()).unwrap();
        assert_eq!(
            json["hello"],
            serde_json::json!({"enrolled": true, "broken": false, "reenrollSuggested": false})
        );
        let status = serde_json::to_value(state.hello_status()).unwrap();
        assert_eq!(status["supported"], true);
        assert_eq!(status["enrolled"], true);
    }
}
