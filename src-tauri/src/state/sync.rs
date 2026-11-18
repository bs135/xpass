//! Sync glue between [`AppState`] and the core engine (T20, decisions S2, S3, S8).
//!
//! - [`AppLocal`] implements the engine's `LocalSide` on the session, the vault
//!   file and `sync-state.json`. Two app-wide counters tell the commit whether the
//!   user changed anything while a sync was talking to GitHub.
//! - The worker (`sync_worker.rs`) decides *when* to sync; this module does the
//!   sync and keeps the status shown in the UI and the tray.

use std::sync::atomic::Ordering;

use xpass_core::sync::{
    blob_sha, push_raw, rebase, sync_once, LocalSide, RemoteStore, Snapshot, SyncError, SyncOutcome, SyncState, Synced,
};

use super::*;
use crate::{github::GitHubStore, sync_state::StoredSyncState, sync_worker::Trigger};

/// What the UI shows about sync (`sync_status`, event `sync://status`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    /// Sync is not set up or turned off.
    #[default]
    Off,
    /// Set up, but no GitHub token is stored on this PC: enter one in Settings.
    NoToken,
    /// Set up; nothing running. `dirty` tells whether changes wait for a push.
    Idle,
    Syncing,
    /// GitHub not reachable; changes are kept and retried.
    Offline,
    /// Token invalid, expired or lacking permission: enter a new token.
    TokenInvalid,
    /// The VaultKey was rotated on another device: the master password is needed.
    NeedsPassword,
    /// The remote file belongs to another vault: sync stopped.
    DifferentVault,
    /// Anything else (rate limit, unreadable remote, I/O); retried later.
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub phase: SyncPhase,
    /// The local vault has changes GitHub has not seen.
    pub dirty: bool,
    /// Unix ms of the last successful sync (or check) in this run or recorded on disk.
    pub last_sync_at: Option<i64>,
    /// Error code for `Offline`, `TokenInvalid`, `NeedsPassword`, `DifferentVault`, `Error`.
    pub error: Option<AppError>,
    /// The master password was changed on another device and adopted here (S5).
    pub password_changed: bool,
}

/// Identifies a snapshot, so the commit can tell what changed meanwhile.
pub struct SnapshotToken {
    changes: u64,
    slot_changes: u64,
    file_sha: String,
}

/// The engine's local side, bound to one remote.
pub struct AppLocal {
    state: AppState,
    remote_id: String,
}

impl From<SyncError> for AppError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Store(e) => e.into(),
            SyncError::Vault(e) => e.into(),
            SyncError::Locked => AppError::Locked,
            SyncError::TooManyConflicts => AppError::SyncFailed,
            SyncError::Local(kind) => AppError::Io { kind: kind.to_string() },
        }
    }
}

impl AppState {
    // ---- change tracking and the worker hook ----

    /// Installs the worker's trigger function. Set once, at startup.
    pub fn set_sync_notifier(&self, notify: impl Fn(Trigger) + Send + Sync + 'static) {
        let _ = self.0.sync_notifier.set(Box::new(notify));
    }

    /// Asks the worker for a sync ("Sync now").
    pub fn request_sync(&self, trigger: Trigger) {
        self.notify(trigger);
    }

    pub(super) fn notify(&self, trigger: Trigger) {
        if let Some(notify) = self.0.sync_notifier.get() {
            notify(trigger);
        }
    }

    /// Records a local change of the vault file. Call it right after the write,
    /// **while still holding the session lock**, so a sync commit never misses it.
    /// `slots`: the password or recovery slot changed (S5: kept over a remote
    /// change until pushed, and pushed right away).
    pub(super) fn changed(&self, slots: bool) {
        self.0.changes.fetch_add(1, Ordering::SeqCst);
        if slots {
            self.0.slot_changes.fetch_add(1, Ordering::SeqCst);
            self.mark_slots_dirty();
            self.notify(Trigger::SlotsChanged);
        } else {
            self.notify(Trigger::Edited);
        }
    }

    fn mark_slots_dirty(&self) {
        let mut stored = sync_state::load(&self.0.sync_state_path);
        stored.state.slots_dirty = true;
        if let Err(e) = sync_state::save(&self.0.sync_state_path, &stored) {
            eprintln!("xpass: saving the sync state failed: {e}");
        }
    }

    // ---- configuration ----

    /// Store for the configured remote, or `None` while sync is off.
    pub fn sync_store(&self) -> AppResult<Option<GitHubStore>> {
        let sync = self.settings().sync;
        if !sync.enabled || !sync.is_configured() {
            return Ok(None);
        }
        match self.0.tokens.get()? {
            Some(token) => Ok(Some(GitHubStore::new(token, &sync)?)),
            None => Ok(None),
        }
    }

    fn sync_enabled(&self) -> bool {
        let sync = self.settings().sync;
        sync.enabled && sync.is_configured()
    }

    fn remote_id(&self) -> String {
        self.settings().sync.remote_id()
    }

    // ---- status ----

    /// Current status, with `dirty` read from disk.
    pub fn sync_status(&self) -> SyncStatus {
        let mut status = lock(&self.0.sync_status).clone();
        if !self.sync_enabled() {
            status.phase = SyncPhase::Off;
            status.error = None;
        } else if matches!(self.0.tokens.get(), Ok(None)) {
            status.phase = SyncPhase::NoToken;
            status.error = None;
        } else if matches!(status.phase, SyncPhase::Off | SyncPhase::NoToken) {
            status.phase = SyncPhase::Idle;
        }
        let stored = sync_state::load(&self.0.sync_state_path);
        status.dirty = match self.0.storage.read() {
            Ok(Some(file)) => stored.for_remote(&self.remote_id()).is_dirty(&file),
            _ => false,
        };
        if status.last_sync_at.is_none() && stored.remote_id == self.remote_id() {
            status.last_sync_at = stored.last_sync_at;
        }
        status
    }

    pub fn set_sync_phase(&self, phase: SyncPhase) -> SyncStatus {
        {
            let mut status = lock(&self.0.sync_status);
            status.phase = phase;
            if phase == SyncPhase::Syncing || phase == SyncPhase::Idle {
                status.error = None;
            }
        }
        self.sync_status()
    }

    /// Records the result of a sync for the UI.
    pub fn finish_sync(&self, result: &Result<SyncOutcome, SyncError>) -> SyncStatus {
        {
            let mut status = lock(&self.0.sync_status);
            match result {
                Ok(_) | Err(SyncError::Locked) => {
                    status.phase = SyncPhase::Idle;
                    status.error = None;
                    if result.is_ok() {
                        status.last_sync_at = Some(now_ms());
                    }
                }
                Err(e) => {
                    status.phase = phase_for(e);
                    status.error = Some(AppError::from(e.clone()));
                }
            }
        }
        self.sync_status()
    }

    /// Hides the "password changed on another device" notice.
    pub fn dismiss_password_changed(&self) {
        lock(&self.0.sync_status).password_changed = false;
    }

    // ---- syncing ----

    /// One sync with `store` (S2). Needs the vault unlocked.
    pub async fn run_sync<S: RemoteStore>(&self, store: &S) -> Result<SyncOutcome, SyncError> {
        let mut local = AppLocal {
            state: self.clone(),
            remote_id: self.remote_id(),
        };
        sync_once(store, &mut local, now_ms()).await
    }

    /// S8: after a lock, uploads the local file as it is when it has unpushed
    /// changes and the remote did not move. Needs no key. Returns whether it pushed.
    pub async fn push_locked<S: RemoteStore>(&self, store: &S) -> Result<bool, SyncError> {
        if lock(&self.0.session).is_some() {
            return Ok(false);
        }
        let Some(file) = self.0.storage.read()? else {
            return Ok(false);
        };
        let remote_id = self.remote_id();
        let stored = sync_state::load(&self.0.sync_state_path);
        let state = stored.for_remote(&remote_id);
        if !state.is_dirty(&file) {
            return Ok(false);
        }
        let Some(sha) = push_raw(store, &file, state.base_sha.as_deref()).await? else {
            return Ok(false);
        };
        // The remote now holds exactly `file`; any later local edit stays dirty.
        self.save_sync_state(remote_id, SyncState::synced(sha, &file))?;
        Ok(true)
    }

    fn save_sync_state(&self, remote_id: String, state: SyncState) -> Result<(), SyncError> {
        let stored = StoredSyncState {
            remote_id,
            state,
            last_sync_at: Some(now_ms()),
        };
        sync_state::save(&self.0.sync_state_path, &stored).map_err(|_| SyncError::Local(std::io::ErrorKind::Other))
    }
}

fn phase_for(e: &SyncError) -> SyncPhase {
    use xpass_core::{sync::StoreError as S, Error as V};
    match e {
        SyncError::Store(S::Offline) => SyncPhase::Offline,
        SyncError::Store(S::Unauthorized | S::Forbidden) => SyncPhase::TokenInvalid,
        SyncError::Vault(V::KeyChanged) => SyncPhase::NeedsPassword,
        SyncError::Vault(V::DifferentVault) => SyncPhase::DifferentVault,
        _ => SyncPhase::Error,
    }
}

impl LocalSide for AppLocal {
    type Token = SnapshotToken;

    fn snapshot(&mut self) -> Result<(Snapshot, SnapshotToken), SyncError> {
        let inner = &self.state.0;
        let guard = lock(&inner.session);
        let session = guard.as_ref().ok_or(SyncError::Locked)?;
        let file = inner
            .storage
            .read()?
            .ok_or(SyncError::Local(std::io::ErrorKind::NotFound))?;
        let state = sync_state::load(&inner.sync_state_path).for_remote(&self.remote_id);
        let snapshot = Snapshot {
            vault: session.vault().clone(),
            dirty: state.is_dirty(&file),
            state,
        };
        let token = SnapshotToken {
            changes: inner.changes.load(Ordering::SeqCst),
            slot_changes: inner.slot_changes.load(Ordering::SeqCst),
            file_sha: blob_sha(&file),
        };
        Ok((snapshot, token))
    }

    /// The `LocalSide` contract: see `xpass_core::sync::LocalSide`.
    fn commit(&mut self, token: SnapshotToken, synced: Synced) -> Result<(), SyncError> {
        let inner = &self.state.0;
        let now = now_ms();
        let mut guard = lock(&inner.session);
        let unchanged = inner.changes.load(Ordering::SeqCst) == token.changes;
        let state = match guard.as_mut() {
            Some(session) if unchanged => {
                inner.storage.write(&synced.bytes, now)?;
                let state = SyncState::synced(synced.base_sha, &synced.bytes);
                session.adopt(synced.vault);
                state
            }
            Some(session) => {
                let slots_changed = inner.slot_changes.load(Ordering::SeqCst) != token.slot_changes;
                let rebased = rebase(session.vault(), &synced.vault, slots_changed);
                session
                    .persist_with(&inner.storage, now, |vault| {
                        *vault = rebased;
                        Ok(())
                    })
                    .map_err(|_| SyncError::Local(std::io::ErrorKind::Other))?;
                session.refresh_clock();
                SyncState {
                    base_sha: Some(synced.base_sha),
                    local_sha: Some(blob_sha(&synced.bytes)),
                    slots_dirty: slots_changed,
                }
            }
            None => {
                let file = inner.storage.read()?;
                if file.as_deref().map(blob_sha).as_deref() != Some(token.file_sha.as_str()) {
                    // Changed and locked meanwhile: leave everything; the next sync merges.
                    return Ok(());
                }
                inner.storage.write(&synced.bytes, now)?;
                SyncState::synced(synced.base_sha, &synced.bytes)
            }
        };
        if synced.password_changed {
            lock(&inner.sync_status).password_changed = true;
        }
        // Still under the session lock, so a local key-slot change (which also
        // writes this file) cannot interleave.
        let saved = self.state.save_sync_state(self.remote_id.clone(), state);
        drop(guard);
        saved
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        future::Future,
        pin::pin,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
    };

    use xpass_core::{
        model::Entry,
        sync::{memory::MemoryStore, StoreError},
    };

    use super::*;
    use crate::{clipboard::fake::FakeClipboard, hello::fake::FakeHello, lock_watch::LockReason};

    const TEST_KDF: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    const PW: &str = "correct horse battery staple";

    /// The memory store never waits, so a busy poll is enough.
    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    struct Device {
        _tmp: tempfile::TempDir,
        paths: Paths,
        state: AppState,
        triggers: Arc<Mutex<Vec<Trigger>>>,
    }

    impl Device {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let paths = Paths::new(tmp.path().join("data"));
            let state = AppState::new(
                paths.clone(),
                TEST_KDF,
                SecureClipboard::new(FakeClipboard::default()),
                Box::new(FakeHello::default()),
            );
            let triggers = Arc::new(Mutex::new(Vec::new()));
            let seen = triggers.clone();
            state.set_sync_notifier(move |t| seen.lock().unwrap().push(t));
            state
                .configure_sync(SyncSettings::from_input("octo/vault", "", "").unwrap())
                .unwrap();
            state.set_sync_token("github_pat_0123456789abcdefghij").unwrap();
            Self {
                _tmp: tmp,
                paths,
                state,
                triggers,
            }
        }

        /// First device: creates the vault.
        fn create() -> Self {
            let d = Self::new();
            d.state.create_vault(PW).unwrap();
            d
        }

        /// Second device that downloaded the remote vault (the real join flow comes in T21).
        fn join(store: &MemoryStore) -> Self {
            let d = Self::new();
            let remote = store.current().unwrap();
            fs::create_dir_all(d.paths.dir()).unwrap();
            fs::write(d.paths.vault(), &remote.bytes).unwrap();
            let stored = StoredSyncState {
                remote_id: d.state.remote_id(),
                state: SyncState::synced(remote.sha, &remote.bytes),
                last_sync_at: None,
            };
            sync_state::save(&d.paths.sync_state(), &stored).unwrap();
            d.state.unlock_password(PW).unwrap();
            d
        }

        fn add(&self, title: &str) {
            self.state
                .edit(|p, ts| {
                    p.entries.push(Entry::new(title, ts));
                    Ok(())
                })
                .unwrap();
        }

        fn titles(&self) -> Vec<String> {
            let mut t: Vec<String> = self
                .state
                .read(|p| Ok(p.entries.iter().map(|e| e.title.clone()).collect()))
                .unwrap();
            t.sort();
            t
        }

        fn sync(&self, store: &MemoryStore) -> Result<SyncOutcome, SyncError> {
            let result = block_on(self.state.run_sync(store));
            self.state.finish_sync(&result);
            result
        }

        fn take_triggers(&self) -> Vec<Trigger> {
            std::mem::take(&mut *self.triggers.lock().unwrap())
        }
    }

    #[test]
    fn two_devices_converge_through_the_store() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.add("GitHub");
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
        let b = Device::join(&store);
        b.sync(&store).unwrap();

        a.add("Mail");
        b.add("Bank");
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
        assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
        assert_eq!(a.titles(), ["Bank", "GitHub", "Mail"]);
        assert_eq!(a.titles(), b.titles());
        for d in [&a, &b] {
            let status = d.state.sync_status();
            assert_eq!(status.phase, SyncPhase::Idle);
            assert!(!status.dirty);
            assert!(status.last_sync_at.is_some());
        }
        // Pulled data is on disk: survives a restart.
        a.state.lock();
        a.state.unlock_password(PW).unwrap();
        assert_eq!(a.titles(), ["Bank", "GitHub", "Mail"]);
    }

    #[test]
    fn status_reports_off_dirty_and_errors() {
        let store = MemoryStore::new();
        let a = Device::create();
        assert_eq!(a.state.sync_status().phase, SyncPhase::Idle);
        assert!(a.state.sync_status().dirty, "never pushed");
        a.sync(&store).unwrap();
        assert!(!a.state.sync_status().dirty);

        a.add("Mail");
        store.set_offline(true);
        assert!(a.sync(&store).is_err());
        let s = a.state.sync_status();
        assert_eq!(
            (s.phase, s.dirty, s.error.clone()),
            (SyncPhase::Offline, true, Some(AppError::SyncOffline))
        );

        store.fail_with(Some(StoreError::Unauthorized));
        a.sync(&store).unwrap_err();
        assert_eq!(a.state.sync_status().phase, SyncPhase::TokenInvalid);

        store.fail_with(None);
        a.sync(&store).unwrap();
        let s = a.state.sync_status();
        assert_eq!((s.phase, s.dirty, s.error), (SyncPhase::Idle, false, None));

        a.state.disconnect_sync().unwrap();
        assert_eq!(a.state.sync_status().phase, SyncPhase::Off);
    }

    #[test]
    fn edit_during_a_sync_is_kept_and_stays_dirty() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.sync(&store).unwrap();
        let b = Device::join(&store);
        b.sync(&store).unwrap();
        b.add("Cloud");
        b.sync(&store).unwrap();

        a.add("Bank");
        let a2 = a.state.clone();
        store.before_next_put(move |_| {
            a2.edit(|p, ts| {
                p.entries.push(Entry::new("Late", ts));
                Ok(())
            })
            .unwrap();
        });
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
        assert_eq!(a.titles(), ["Bank", "Cloud", "Late"]);
        assert!(a.state.sync_status().dirty);
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
        b.sync(&store).unwrap();
        assert_eq!(b.titles(), ["Bank", "Cloud", "Late"]);
    }

    #[test]
    fn lock_during_a_sync_still_records_the_push() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.add("Bank");
        let a2 = a.state.clone();
        store.before_next_put(move |_| {
            a2.lock();
        });
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
        assert_eq!(fs::read(a.paths.vault()).unwrap(), store.current().unwrap().bytes);
        assert!(!a.state.sync_status().dirty);
    }

    #[test]
    fn keyless_push_after_lock() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.sync(&store).unwrap();
        let b = Device::join(&store);
        b.sync(&store).unwrap();

        a.add("Mail");
        assert!(
            !block_on(a.state.push_locked(&store)).unwrap(),
            "unlocked: the normal sync does it"
        );
        a.state.lock_with(LockReason::Manual);
        assert!(block_on(a.state.push_locked(&store)).unwrap());
        assert!(!a.state.sync_status().dirty);
        assert!(!block_on(a.state.push_locked(&store)).unwrap(), "nothing left to push");
        assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
        assert_eq!(b.titles(), ["Mail"]);

        // Remote moved on: nothing is pushed, the next unlock merges.
        b.add("Cloud");
        b.sync(&store).unwrap();
        a.state.unlock_password(PW).unwrap();
        a.add("Bank");
        a.state.lock_with(LockReason::Idle);
        assert!(!block_on(a.state.push_locked(&store)).unwrap());
        a.state.unlock_password(PW).unwrap();
        assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
        assert_eq!(a.titles(), ["Bank", "Cloud", "Mail"]);
    }

    #[test]
    fn password_change_elsewhere_is_adopted_and_reported() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.sync(&store).unwrap();
        let b = Device::join(&store);
        b.sync(&store).unwrap();

        let new_pw = "a brand new master password 42";
        a.state.change_password(Some(PW), new_pw).unwrap();
        assert!(sync_state::load(&a.paths.sync_state()).state.slots_dirty);
        a.sync(&store).unwrap();
        assert!(!sync_state::load(&a.paths.sync_state()).state.slots_dirty);
        assert!(!a.state.sync_status().password_changed);

        assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
        assert!(b.state.sync_status().password_changed);
        b.state.dismiss_password_changed();
        assert!(!b.state.sync_status().password_changed);
        b.state.lock();
        assert_eq!(b.state.unlock_password(PW).unwrap_err(), AppError::BadCredentials);
        b.state.unlock_password(new_pw).unwrap();
    }

    #[test]
    fn a_missing_token_is_reported() {
        let a = Device::create();
        a.state.0.tokens.delete().unwrap();
        let s = a.state.sync_status();
        assert_eq!((s.phase, s.error), (SyncPhase::NoToken, None));

        a.state.set_sync_token("github_pat_0123456789abcdefghij").unwrap();
        assert_eq!(a.state.sync_status().phase, SyncPhase::Idle);

        a.state.0.tokens.delete().unwrap();
        a.state.disconnect_sync().unwrap();
        assert_eq!(a.state.sync_status().phase, SyncPhase::Off);
    }

    #[test]
    fn sync_needs_the_vault_unlocked() {
        let store = MemoryStore::new();
        let a = Device::create();
        a.state.lock();
        assert_eq!(a.sync(&store).unwrap_err(), SyncError::Locked);
        assert_eq!(a.state.sync_status().phase, SyncPhase::Idle, "not an error");
    }

    #[test]
    fn local_changes_notify_the_worker() {
        let a = Device::create();
        a.take_triggers();
        a.add("x");
        let pw2 = "another strong password 1234";
        a.state.change_password(Some(PW), pw2).unwrap();
        a.state.lock_with(LockReason::Manual);
        a.state.unlock_password(pw2).unwrap();
        a.state.lock_with(LockReason::Exit);
        assert_eq!(
            a.take_triggers(),
            [
                Trigger::Edited,
                Trigger::SlotsChanged,
                Trigger::Locked,
                Trigger::Unlocked
            ],
            "exit pushes on its own"
        );
        a.state.unlock_password(pw2).unwrap();
        a.take_triggers();
        a.state.disconnect_sync().unwrap();
        assert_eq!(a.take_triggers(), [Trigger::ConfigChanged]);
    }
}
