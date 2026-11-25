//! The sync worker (S8, S9, D8): one tokio task that decides *when* to sync.
//!
//! - on unlock, on "Sync now", right after a local password / kit change, and when
//!   the sync setup changes: sync now;
//! - 10 s after the last edit (debounced);
//! - every 5 min while unlocked;
//! - offline or a transient error: retry with backoff (30 s, 1, 2, 4, 5 min);
//!   token / vault problems wait for the user;
//! - on lock: push the local file without the key if the remote did not move
//!   (8 s timeout); on exit the same, blocking ([`push_on_exit`]).
//!
//! Only this task syncs, so two syncs never overlap. Status changes are emitted as
//! `sync://status`; a sync that brought remote changes emits `vault://changed`.

use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use xpass_core::sync::{StoreError, SyncError, SyncOutcome};

use crate::{
    state::{AppState, SyncStatus, VaultStatus},
    tray,
};

pub const EVENT_STATUS: &str = "sync://status";
pub const EVENT_VAULT_CHANGED: &str = "vault://changed";

pub const DEBOUNCE: Duration = Duration::from_secs(10);
pub const PERIOD: Duration = Duration::from_secs(5 * 60);
pub const BACKOFF_MIN: Duration = Duration::from_secs(30);
pub const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
pub const LOCK_PUSH_TIMEOUT: Duration = Duration::from_secs(8);

/// Something that may call for a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Unlocked,
    Edited,
    /// The password or recovery slot changed locally (S5).
    SlotsChanged,
    /// "Sync now" (Settings, header, tray).
    Manual,
    Locked,
    /// Repository, branch, path or token changed, or sync was turned off.
    ConfigChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Sync,
    PushLocked,
}

/// Deadlines of the worker. Pure, so the timing rules are unit-tested.
#[derive(Debug, Default, Clone)]
pub struct Schedule {
    debounce_at: Option<Instant>,
    periodic_at: Option<Instant>,
    retry_at: Option<Instant>,
    backoff: Duration,
}

impl Schedule {
    pub fn next_wake(&self) -> Option<Instant> {
        [self.debounce_at, self.periodic_at, self.retry_at]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn on_trigger(&mut self, trigger: Trigger, now: Instant) -> Action {
        match trigger {
            Trigger::Unlocked | Trigger::Manual | Trigger::SlotsChanged => {
                self.debounce_at = None;
                self.retry_at = None;
                Action::Sync
            }
            Trigger::ConfigChanged => {
                *self = Self::default();
                Action::Sync
            }
            Trigger::Edited => {
                self.debounce_at = Some(now + DEBOUNCE);
                Action::Nothing
            }
            Trigger::Locked => {
                *self = Self {
                    backoff: self.backoff,
                    ..Self::default()
                };
                Action::PushLocked
            }
        }
    }

    /// A deadline passed.
    pub fn on_wake(&mut self, now: Instant) -> Action {
        let mut due = false;
        for slot in [&mut self.debounce_at, &mut self.periodic_at, &mut self.retry_at] {
            if slot.is_some_and(|at| at <= now) {
                *slot = None;
                due = true;
            }
        }
        if due {
            Action::Sync
        } else {
            Action::Nothing
        }
    }

    /// Plans the next sync after one finished. `still_dirty`: local changes made
    /// during the sync wait for a push.
    pub fn after_sync(&mut self, result: &Result<SyncOutcome, SyncError>, still_dirty: bool, now: Instant) {
        self.retry_at = None;
        match result {
            Ok(_) => {
                self.backoff = Duration::ZERO;
                self.periodic_at = Some(now + PERIOD);
                if still_dirty {
                    self.debounce_at = Some(now + DEBOUNCE);
                }
            }
            Err(e) if waits_for_user(e) => {
                self.periodic_at = None;
                self.debounce_at = None;
            }
            Err(SyncError::Store(StoreError::RateLimited { retry_after_s })) => {
                let wait = retry_after_s.map_or(BACKOFF_MAX, Duration::from_secs).max(BACKOFF_MIN);
                self.periodic_at = None;
                self.retry_at = Some(now + wait);
            }
            Err(_) => {
                self.backoff = (self.backoff * 2).clamp(BACKOFF_MIN, BACKOFF_MAX);
                self.periodic_at = None;
                self.retry_at = Some(now + self.backoff);
            }
        }
    }
}

/// Errors only the user can fix (token, password, another vault), or a locked
/// vault (the next unlock syncs).
fn waits_for_user(e: &SyncError) -> bool {
    use xpass_core::Error as V;
    matches!(
        e,
        SyncError::Locked
            | SyncError::Store(StoreError::Unauthorized | StoreError::Forbidden | StoreError::RepoNotFound)
            | SyncError::Vault(V::KeyChanged | V::DifferentVault | V::BadCredentials | V::UnsupportedVersion(_))
    )
}

/// Starts the worker and connects it to the app state.
pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::unbounded_channel();
    app.state::<AppState>().set_sync_notifier(move |trigger| {
        let _ = tx.send(trigger);
    });
    tauri::async_runtime::spawn(run(app, rx));
}

async fn run(app: AppHandle, mut rx: mpsc::UnboundedReceiver<Trigger>) {
    let state = app.state::<AppState>().inner().clone();
    let mut schedule = Schedule::default();
    publish(&app, &state.sync_status());
    loop {
        let action = match schedule.next_wake() {
            Some(at) => tokio::select! {
                trigger = rx.recv() => match trigger {
                    Some(t) => schedule.on_trigger(t, Instant::now()),
                    None => return,
                },
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(at)) => schedule.on_wake(Instant::now()),
            },
            None => match rx.recv().await {
                Some(t) => schedule.on_trigger(t, Instant::now()),
                None => return,
            },
        };
        match action {
            Action::Nothing => {}
            Action::Sync => sync(&app, &state, &mut schedule).await,
            Action::PushLocked => {
                push_locked(&state).await;
                publish(&app, &state.sync_status());
            }
        }
    }
}

async fn sync(app: &AppHandle, state: &AppState, schedule: &mut Schedule) {
    let store = match state.sync_store() {
        Ok(Some(store)) => store,
        // Off, or Credential Manager unavailable: nothing to do until the setup changes.
        Ok(None) | Err(_) => {
            *schedule = Schedule::default();
            publish(app, &state.sync_status());
            return;
        }
    };
    if state.status() != VaultStatus::Unlocked {
        publish(app, &state.sync_status());
        return;
    }
    publish(app, &state.set_sync_phase(crate::state::SyncPhase::Syncing));
    let result = state.run_sync(&store).await;
    let status = state.finish_sync(&result);
    schedule.after_sync(&result, status.dirty, Instant::now());
    if matches!(result, Ok(SyncOutcome::Pulled | SyncOutcome::Merged)) {
        let _ = app.emit(EVENT_VAULT_CHANGED, ());
    }
    publish(app, &status);
}

/// S8 on lock. Errors are ignored: the changes stay local and the next unlock syncs.
pub(crate) async fn push_locked(state: &AppState) {
    if let Ok(Some(store)) = state.sync_store() {
        let _ = tokio::time::timeout(LOCK_PUSH_TIMEOUT, state.push_locked(&store)).await;
    }
}

/// S8 on app exit (the vault is already locked): blocks up to 8 s.
pub fn push_on_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let state = state.inner().clone();
    tauri::async_runtime::block_on(push_locked(&state));
}

fn publish(app: &AppHandle, status: &SyncStatus) {
    let _ = app.emit(EVENT_STATUS, status);
    tray::show_sync_status(app, status);
}

#[cfg(test)]
mod tests {
    use xpass_core::Error as V;

    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn edits_are_debounced() {
        let now = t0();
        let mut s = Schedule::default();
        assert_eq!(s.on_trigger(Trigger::Edited, now), Action::Nothing);
        assert_eq!(
            s.on_trigger(Trigger::Edited, now + Duration::from_secs(5)),
            Action::Nothing
        );
        assert_eq!(
            s.next_wake(),
            Some(now + Duration::from_secs(15)),
            "deadline moves with each edit"
        );
        assert_eq!(s.on_wake(now + Duration::from_secs(14)), Action::Nothing);
        assert_eq!(s.on_wake(now + Duration::from_secs(15)), Action::Sync);
        assert_eq!(s.next_wake(), None);
    }

    #[test]
    fn immediate_triggers() {
        let now = t0();
        for t in [
            Trigger::Unlocked,
            Trigger::Manual,
            Trigger::SlotsChanged,
            Trigger::ConfigChanged,
        ] {
            let mut s = Schedule::default();
            s.on_trigger(Trigger::Edited, now);
            assert_eq!(s.on_trigger(t, now), Action::Sync, "{t:?}");
            assert_eq!(s.next_wake(), None, "pending debounce folded into this sync");
        }
    }

    #[test]
    fn success_schedules_the_periodic_sync() {
        let now = t0();
        let mut s = Schedule::default();
        s.after_sync(&Ok(SyncOutcome::Pushed), false, now);
        assert_eq!(s.next_wake(), Some(now + PERIOD));
        s.after_sync(&Ok(SyncOutcome::Merged), true, now);
        assert_eq!(s.next_wake(), Some(now + DEBOUNCE), "edits made during the sync");
    }

    #[test]
    fn offline_backs_off_and_recovers() {
        let now = t0();
        let mut s = Schedule::default();
        let offline = Err(SyncError::Store(StoreError::Offline));
        let mut waits = Vec::new();
        for _ in 0..6 {
            s.after_sync(&offline, true, now);
            waits.push(s.next_wake().unwrap() - now);
        }
        let secs: Vec<u64> = waits.iter().map(Duration::as_secs).collect();
        assert_eq!(secs, [30, 60, 120, 240, 300, 300]);
        s.after_sync(&Ok(SyncOutcome::Pushed), false, now);
        s.after_sync(&offline, true, now);
        assert_eq!(s.next_wake(), Some(now + BACKOFF_MIN), "backoff reset after success");
    }

    #[test]
    fn rate_limit_waits_as_told() {
        let now = t0();
        let mut s = Schedule::default();
        s.after_sync(
            &Err(SyncError::Store(StoreError::RateLimited {
                retry_after_s: Some(600),
            })),
            false,
            now,
        );
        assert_eq!(s.next_wake(), Some(now + Duration::from_secs(600)));
        s.after_sync(
            &Err(SyncError::Store(StoreError::RateLimited { retry_after_s: Some(1) })),
            false,
            now,
        );
        assert_eq!(s.next_wake(), Some(now + BACKOFF_MIN));
    }

    #[test]
    fn user_problems_stop_automatic_retries() {
        let now = t0();
        for e in [
            SyncError::Store(StoreError::Unauthorized),
            SyncError::Store(StoreError::Forbidden),
            SyncError::Vault(V::KeyChanged),
            SyncError::Vault(V::DifferentVault),
            SyncError::Locked,
        ] {
            let mut s = Schedule::default();
            s.on_trigger(Trigger::Edited, now);
            s.after_sync(&Err(e.clone()), true, now);
            assert_eq!(s.next_wake(), None, "{e:?}");
        }
    }

    #[test]
    fn lock_cancels_timers_and_pushes() {
        let now = t0();
        let mut s = Schedule::default();
        s.after_sync(&Ok(SyncOutcome::Pushed), false, now);
        s.on_trigger(Trigger::Edited, now);
        assert_eq!(s.on_trigger(Trigger::Locked, now), Action::PushLocked);
        assert_eq!(s.next_wake(), None);
    }
}
