//! Sync engine tests with the in-memory store (ported from the GitHub sync spike,
//! `docs/spikes/github-sync.md`, plus the v0.4 cases: key slots per device, edits during a sync, crash safety, ...).

use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, Waker},
};

use rand::{rngs::OsRng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use uuid::Uuid;
use xpass_core::{
    crypto::KdfParams,
    format::SlotKind,
    maintenance::TOMBSTONE_TTL_MS,
    model::{Entry, Payload, HISTORY_NOTE_CONFLICT},
    sync::{
        blob_sha, memory::MemoryStore, push_raw, rebase, resolve_key_change, sync_once, LocalSide, Snapshot,
        StoreError, SyncError, SyncOutcome, SyncState, Synced,
    },
    Error, Unlock, UnlockedVault,
};

const KDF: KdfParams = KdfParams {
    m_kib: 8 * 1024,
    t: 1,
    p: 1,
};
const PW: &[u8] = b"pw";
const NOW: i64 = 1_800_000_000_000;

/// Drives a future that never waits (the memory store is always ready).
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn entry(id: u128, title: &str, updated_at: i64) -> Entry {
    let mut e = Entry::new(title, 1_000);
    e.id = Uuid::from_u128(id);
    e.updated_at = updated_at;
    e
}

fn titles(p: &Payload) -> Vec<String> {
    p.entries
        .iter()
        .map(|e| {
            if e.deleted {
                format!("~{}", e.id.as_u128())
            } else {
                e.title.clone()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// A device: local file + SyncState + optional unlocked session, implementing
// the LocalSide contract the way the app does.
// ---------------------------------------------------------------------------

struct Session {
    vault: UnlockedVault,
    /// Bumped on every local change.
    generation: u64,
    /// Bumped on every local key-slot change (password, recovery).
    slots_generation: u64,
}

struct Inner {
    file: Vec<u8>,
    state: SyncState,
    session: Option<Session>,
    password_changed: bool,
}

#[derive(Clone)]
struct Device(Arc<Mutex<Inner>>);

struct Token {
    generation: u64,
    slots_generation: u64,
    file_sha: String,
}

impl Device {
    fn new(vault: UnlockedVault, state: SyncState) -> Self {
        let mut vault = vault;
        let file = vault.save(&mut OsRng).unwrap();
        Self(Arc::new(Mutex::new(Inner {
            file,
            state,
            session: Some(Session {
                vault,
                generation: 0,
                slots_generation: 0,
            }),
            password_changed: false,
        })))
    }

    fn create() -> Self {
        let (vault, _) = UnlockedVault::create(&mut OsRng, PW, KDF).unwrap();
        Self::new(vault, SyncState::default())
    }

    /// New device joining the vault on the remote.
    fn join(store: &MemoryStore, password: &[u8]) -> Self {
        let remote = store.current().expect("remote vault exists");
        let mut vault = xpass_core::sync::join(&remote.bytes, password).unwrap();
        let file = vault.save(&mut OsRng).unwrap();
        let state = SyncState::synced(remote.sha, &file);
        let device = Self::new(vault, SyncState::default());
        let mut inner = device.inner();
        inner.file = file;
        inner.state = state;
        drop(inner);
        device
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }

    fn change(&self, slots: bool, f: impl FnOnce(&mut UnlockedVault)) {
        let mut inner = self.inner();
        let session = inner.session.as_mut().expect("unlocked");
        f(&mut session.vault);
        session.generation += 1;
        if slots {
            session.slots_generation += 1;
        }
        let file = session.vault.save(&mut OsRng).unwrap();
        inner.file = file;
        if slots {
            inner.state.slots_dirty = true;
        }
    }

    fn edit(&self, f: impl FnOnce(&mut Payload)) {
        self.change(false, |v| f(&mut v.payload));
    }

    fn change_password(&self, new: &[u8]) {
        self.change(true, |v| v.change_password(&mut OsRng, new, KDF).unwrap());
    }

    fn lock(&self) {
        self.inner().session = None;
    }

    fn unlock(&self, how: Unlock<'_>) -> Result<(), Error> {
        let mut inner = self.inner();
        let vault = UnlockedVault::unlock(&inner.file, how)?;
        inner.session = Some(Session {
            vault,
            generation: 0,
            slots_generation: 0,
        });
        Ok(())
    }

    fn titles(&self) -> Vec<String> {
        titles(&self.inner().session.as_ref().expect("unlocked").vault.payload)
    }

    fn vault(&self) -> UnlockedVault {
        self.inner().session.as_ref().expect("unlocked").vault.clone()
    }

    fn file(&self) -> Vec<u8> {
        self.inner().file.clone()
    }

    fn state(&self) -> SyncState {
        self.inner().state.clone()
    }

    fn dirty(&self) -> bool {
        let inner = self.inner();
        inner.state.is_dirty(&inner.file)
    }

    fn take_password_changed(&self) -> bool {
        std::mem::take(&mut self.inner().password_changed)
    }

    fn sync(&self, store: &MemoryStore) -> Result<SyncOutcome, SyncError> {
        block_on(sync_once(store, &mut self.clone(), NOW))
    }
}

impl LocalSide for Device {
    type Token = Token;

    fn snapshot(&mut self) -> Result<(Snapshot, Token), SyncError> {
        let inner = self.inner();
        let session = inner.session.as_ref().ok_or(SyncError::Locked)?;
        let snap = Snapshot {
            vault: session.vault.clone(),
            state: inner.state.clone(),
            dirty: inner.state.is_dirty(&inner.file),
        };
        let token = Token {
            generation: session.generation,
            slots_generation: session.slots_generation,
            file_sha: blob_sha(&inner.file),
        };
        Ok((snap, token))
    }

    fn commit(&mut self, token: Token, synced: Synced) -> Result<(), SyncError> {
        let mut guard = self.inner();
        let inner = &mut *guard;
        inner.password_changed |= synced.password_changed;
        match inner.session.as_mut() {
            Some(session) if session.generation == token.generation => {
                session.vault = synced.vault;
                inner.state = SyncState::synced(synced.base_sha, &synced.bytes);
                inner.file = synced.bytes;
            }
            Some(session) => {
                let slots_changed = session.slots_generation != token.slots_generation;
                session.vault = rebase(&session.vault, &synced.vault, slots_changed);
                inner.file = session.vault.save(&mut OsRng).unwrap();
                inner.state = SyncState {
                    base_sha: Some(synced.base_sha),
                    local_sha: Some(blob_sha(&synced.bytes)),
                    slots_dirty: slots_changed,
                };
            }
            None if blob_sha(&inner.file) == token.file_sha => {
                inner.state = SyncState::synced(synced.base_sha, &synced.bytes);
                inner.file = synced.bytes;
            }
            None => {}
        }
        Ok(())
    }
}

/// Device A created the vault with one entry and pushed it; device B joined.
fn two_devices(store: &MemoryStore) -> (Device, Device) {
    let a = Device::create();
    a.edit(|p| p.entries.push(entry(1, "GitHub", 10)));
    assert_eq!(a.sync(store).unwrap(), SyncOutcome::Pushed);
    let b = Device::join(store, PW);
    (a, b)
}

// ---------------------------------------------------------------------------
// Basics
// ---------------------------------------------------------------------------

#[test]
fn blob_sha_matches_git() {
    // `printf 'hello\n' | git hash-object --stdin`
    assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
}

#[test]
fn dirty_is_derived_from_the_local_file() {
    assert!(SyncState::default().is_dirty(b"x"), "never synced");
    let state = SyncState::synced("abc".into(), b"x");
    assert!(!state.is_dirty(b"x"));
    assert!(state.is_dirty(b"y"));
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["baseSha"], "abc");
    assert_eq!(serde_json::from_value::<SyncState>(json).unwrap(), state);
}

#[test]
fn first_push_then_second_sync_is_a_no_op() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    assert_eq!(b.titles(), ["GitHub"]);
    assert!(!a.dirty() && !b.dirty());
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::UpToDate);
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::UpToDate);
    assert_eq!(store.puts(), 1);
}

#[test]
fn sync_needs_an_unlocked_vault() {
    let store = MemoryStore::new();
    let a = Device::create();
    a.lock();
    assert_eq!(a.sync(&store).unwrap_err(), SyncError::Locked);
}

// ---------------------------------------------------------------------------
// Merging (spike tests)
// ---------------------------------------------------------------------------

#[test]
fn disjoint_edits_merge() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    b.edit(|p| p.entries.push(entry(3, "Bank", 20)));
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(a.titles(), ["GitHub", "Mail", "Bank"]);
    assert_eq!(a.titles(), b.titles());
    assert!(!a.dirty() && !b.dirty());
}

#[test]
fn same_entry_newer_edit_wins_on_both_devices_and_loser_goes_to_history() {
    for b_first in [false, true] {
        let store = MemoryStore::new();
        let (a, b) = two_devices(&store);
        a.edit(|p| {
            p.entries[0].title = "from A".into();
            p.entries[0].password = "pw-a".into();
            p.entries[0].updated_at = 30;
        });
        b.edit(|p| {
            p.entries[0].title = "from B".into();
            p.entries[0].password = "pw-b".into();
            p.entries[0].updated_at = 40;
        });
        let (first, second) = if b_first { (&b, &a) } else { (&a, &b) };
        first.sync(&store).unwrap();
        second.sync(&store).unwrap();
        first.sync(&store).unwrap();
        for d in [&a, &b] {
            let v = d.vault();
            assert_eq!(titles(&v.payload), ["from B"]);
            let history = &v.payload.entries[0].history;
            assert!(
                history
                    .iter()
                    .any(|h| h.password == "pw-a" && h.note.as_deref() == Some(HISTORY_NOTE_CONFLICT)),
                "D1: the losing password is kept"
            );
        }
    }
}

#[test]
fn delete_is_not_resurrected_by_stale_device() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    // Recent tombstone: old ones are purged on push (D3).
    a.edit(|p| p.entries[0].delete(NOW - 1_000));
    a.sync(&store).unwrap();
    b.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
    a.sync(&store).unwrap();
    assert_eq!(a.titles(), ["~1", "Mail"]);
    assert_eq!(b.titles(), ["~1", "Mail"]);
}

#[test]
fn race_between_get_and_put_is_retried() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    b.edit(|p| p.entries.push(entry(3, "Cloud", 20)));
    let b_bytes = b.vault().save(&mut OsRng).unwrap();
    a.edit(|p| p.entries.push(entry(2, "Bank", 20)));
    // B pushes right between A's GET and PUT.
    store.before_next_put(move |s| s.force(&b_bytes));
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(a.titles(), ["GitHub", "Bank", "Cloud"]);
    assert_eq!(store.puts(), 2, "the first PUT conflicted, the retry succeeded");
}

#[test]
fn gives_up_after_repeated_conflicts() {
    let store = Arc::new(MemoryStore::new());
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Bank", 20)));
    let mut other = b.vault();
    let copies: Vec<Vec<u8>> = (0..6).map(|_| other.save(&mut OsRng).unwrap()).collect();
    fn arm(store: &MemoryStore, mut copies: Vec<Vec<u8>>) {
        if let Some(bytes) = copies.pop() {
            store.before_next_put(move |s| {
                s.force(&bytes);
                arm(s, copies);
            });
        }
    }
    arm(&store, copies);
    assert_eq!(a.sync(&store).unwrap_err(), SyncError::TooManyConflicts);
    assert!(a.dirty(), "local edit kept for the next try");
}

// ---------------------------------------------------------------------------
// Offline and errors
// ---------------------------------------------------------------------------

#[test]
fn offline_edits_are_kept_and_pushed_later() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    store.set_offline(true);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    assert_eq!(a.sync(&store).unwrap_err(), SyncError::Store(StoreError::Offline));
    // App restart: the local file and state survive.
    a.lock();
    a.unlock(Unlock::Password(PW)).unwrap();
    assert!(a.dirty());
    store.set_offline(false);
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(b.titles(), ["GitHub", "Mail"]);
}

#[test]
fn store_errors_are_reported_and_nothing_changes() {
    let store = MemoryStore::new();
    let (a, _b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    let file = a.file();
    for e in [
        StoreError::Unauthorized,
        StoreError::Forbidden,
        StoreError::RateLimited {
            retry_after_s: Some(60),
        },
        StoreError::RepoNotFound,
    ] {
        store.fail_with(Some(e));
        assert_eq!(a.sync(&store).unwrap_err(), SyncError::Store(e));
        assert_eq!(a.file(), file);
        assert!(a.dirty());
    }
}

#[test]
fn unreadable_remote_is_read_again_once() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    b.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    b.sync(&store).unwrap();

    store.serve_once(b"<html>oops</html>".to_vec());
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(store.pauses(), 1);

    b.edit(|p| p.entries.push(entry(3, "Bank", 30)));
    b.sync(&store).unwrap();
    store.serve_once(b"garbage".to_vec());
    store.serve_once(b"garbage again".to_vec());
    let err = a.sync(&store).unwrap_err();
    assert!(matches!(err, SyncError::Vault(Error::Format(_))), "{err:?}");
    assert_eq!(store.pauses(), 2, "only one retry per sync");
}

#[test]
fn remote_from_another_vault_is_refused() {
    let store = MemoryStore::new();
    let (a, _b) = two_devices(&store);
    let mut other = UnlockedVault::create(&mut OsRng, PW, KDF).unwrap().0;
    store.force(&other.save(&mut OsRng).unwrap());
    let before = store.current();
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    assert_eq!(a.sync(&store).unwrap_err(), SyncError::Vault(Error::DifferentVault));
    assert_eq!(store.current(), before, "nothing overwritten");
    assert!(a.dirty());
}

// ---------------------------------------------------------------------------
// Concurrency with local changes (S2) and crash safety (S3)
// ---------------------------------------------------------------------------

#[test]
fn edit_during_a_sync_is_kept_and_pushed_next_time() {
    let store = Arc::new(MemoryStore::new());
    let (a, b) = two_devices(&store);
    b.edit(|p| p.entries.push(entry(3, "Cloud", 20)));
    b.sync(&store).unwrap();

    a.edit(|p| p.entries.push(entry(2, "Bank", 20)));
    let a2 = a.clone();
    store.before_next_put(move |_| a2.edit(|p| p.entries.push(entry(4, "Late", 30))));
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(
        a.titles(),
        ["GitHub", "Bank", "Cloud", "Late"],
        "sorted by created_at, id"
    );
    assert!(a.dirty(), "the late edit is not on the remote yet");
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
    b.sync(&store).unwrap();
    assert_eq!(b.titles(), a.titles());
}

#[test]
fn lock_during_a_sync_still_records_the_push() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    b.edit(|p| p.entries.push(entry(3, "Cloud", 20)));
    b.sync(&store).unwrap();
    a.edit(|p| p.entries.push(entry(2, "Bank", 20)));
    let a2 = a.clone();
    store.before_next_put(move |_| a2.lock());
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(
        a.file(),
        store.current().unwrap().bytes,
        "merged file written although locked"
    );
    assert!(!a.dirty());
    a.unlock(Unlock::Password(PW)).unwrap();
    assert_eq!(a.titles(), ["GitHub", "Bank", "Cloud"]);
}

#[test]
fn crash_between_vault_and_state_write_costs_one_push_not_an_edit() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    a.sync(&store).unwrap();
    let old_state = b.state();
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
    // Crash: the vault file was replaced, the state file was not.
    b.inner().state = old_state;
    assert!(b.dirty());
    let puts = store.puts();
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(store.puts(), puts + 1);
    assert_eq!(b.titles(), ["GitHub", "Mail"]);
    a.sync(&store).unwrap();
    assert_eq!(a.titles(), ["GitHub", "Mail"]);
}

#[test]
fn old_tombstones_are_purged_before_a_push() {
    let store = MemoryStore::new();
    let (a, _b) = two_devices(&store);
    a.edit(|p| {
        p.entries.push(entry(2, "old", 5));
        p.entries[1].delete(NOW - TOMBSTONE_TTL_MS - 1);
        p.entries.push(entry(3, "recent", 5));
        p.entries[2].delete(NOW - 1_000);
    });
    a.sync(&store).unwrap();
    assert_eq!(a.titles(), ["GitHub", "~3"]);
}

// ---------------------------------------------------------------------------
// Key slots across devices (S4, S5, S7)
// ---------------------------------------------------------------------------

#[test]
fn password_change_on_other_device_is_adopted_and_reported() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.change_password(b"new-pw");
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);
    assert!(!a.take_password_changed());
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert!(b.take_password_changed());
    b.lock();
    assert_eq!(b.unlock(Unlock::Password(PW)).unwrap_err(), Error::BadCredentials);
    b.unlock(Unlock::Password(b"new-pw")).unwrap();
}

#[test]
fn unpushed_local_password_change_wins_over_a_remote_one() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.change_password(b"pw-a");
    a.sync(&store).unwrap();
    b.change_password(b"pw-b");
    b.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
    assert!(!b.take_password_changed());
    assert!(!b.state().slots_dirty);
    a.sync(&store).unwrap();
    assert!(a.take_password_changed());
    for d in [&a, &b] {
        d.lock();
        assert_eq!(d.unlock(Unlock::Password(b"pw-a")).unwrap_err(), Error::BadCredentials);
        d.unlock(Unlock::Password(b"pw-b")).unwrap();
        assert_eq!(d.titles(), ["GitHub", "Mail"]);
    }
}

#[test]
fn hello_slots_stay_on_their_own_device() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    let mut rng = ChaCha20Rng::seed_from_u64(1);
    let salt_a = UnlockedVault::new_hello_salt(&mut rng);
    let salt_b = UnlockedVault::new_hello_salt(&mut rng);
    a.change(false, |v| v.set_hello_slot(&mut OsRng, salt_a, b"signature of A"));
    a.sync(&store).unwrap();
    b.sync(&store).unwrap();
    assert!(
        !b.vault().slot_kinds().contains(&SlotKind::Hello),
        "A's Hello slot is not adopted"
    );

    b.change(false, |v| v.set_hello_slot(&mut OsRng, salt_b, b"signature of B"));
    b.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    b.sync(&store).unwrap();
    a.edit(|p| p.entries.push(entry(3, "Bank", 30)));
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
    b.sync(&store).unwrap();

    for (d, sig) in [(&a, &b"signature of A"[..]), (&b, &b"signature of B"[..])] {
        d.lock();
        d.unlock(Unlock::Hello { signature: sig }).unwrap();
        assert_eq!(d.titles(), ["GitHub", "Mail", "Bank"]);
    }
}

#[test]
fn join_drops_the_remote_hello_slot() {
    let store = MemoryStore::new();
    let a = Device::create();
    let salt = UnlockedVault::new_hello_salt(&mut OsRng);
    a.change(false, |v| v.set_hello_slot(&mut OsRng, salt, b"signature of A"));
    a.sync(&store).unwrap();
    let b = Device::join(&store, PW);
    assert_eq!(b.vault().slot_kinds(), [SlotKind::Password, SlotKind::Recovery]);
    assert_eq!(
        xpass_core::sync::join(&store.current().unwrap().bytes, b"wrong").unwrap_err(),
        Error::BadCredentials
    );
}

#[test]
fn rotation_elsewhere_needs_the_password_and_keeps_local_edits() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    let salt = UnlockedVault::new_hello_salt(&mut OsRng);
    b.change(false, |v| v.set_hello_slot(&mut OsRng, salt, b"signature of B"));
    b.sync(&store).unwrap();

    // S6: rotation runs right after a sync, so the remote is at A's base.
    a.sync(&store).unwrap();
    a.change(true, |v| {
        v.rotate(&mut OsRng, b"rotated-pw", KDF).unwrap();
    });
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pushed);

    b.edit(|p| p.entries.push(entry(2, "Offline edit on B", 20)));
    let file = b.file();
    assert_eq!(b.sync(&store).unwrap_err(), SyncError::Vault(Error::KeyChanged));
    assert_eq!(b.file(), file, "nothing changed locally");
    assert!(b.dirty());

    let wrong = block_on(resolve_key_change(&store, &mut b.clone(), b"old?", NOW)).unwrap_err();
    assert_eq!(wrong, SyncError::Vault(Error::BadCredentials));
    let outcome = block_on(resolve_key_change(&store, &mut b.clone(), b"rotated-pw", NOW)).unwrap();
    assert_eq!(outcome, SyncOutcome::Merged);
    assert!(b.vault().same_key(&a.vault()), "B adopted the new key");
    assert!(
        !b.vault().slot_kinds().contains(&SlotKind::Hello),
        "old Hello slot dropped"
    );
    assert!(!b.dirty());

    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(a.titles(), ["GitHub", "Offline edit on B"]);
    for d in [&a, &b] {
        d.lock();
        d.unlock(Unlock::Password(b"rotated-pw")).unwrap();
    }
}

#[test]
fn resolve_key_change_without_a_key_change_is_a_normal_sync() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    a.sync(&store).unwrap();
    let outcome = block_on(resolve_key_change(&store, &mut b.clone(), b"unused", NOW)).unwrap();
    assert_eq!(outcome, SyncOutcome::Pulled);
}

#[test]
fn rebase_keeps_local_slots_only_when_the_key_matches() {
    let (mut current, _) = UnlockedVault::create(&mut OsRng, PW, KDF).unwrap();
    current.payload.entries.push(entry(1, "local", 10));
    let mut synced = current.clone();
    synced.payload.entries.push(entry(2, "synced", 10));
    let salt = UnlockedVault::new_hello_salt(&mut OsRng);
    current.set_hello_slot(&mut OsRng, salt, b"sig");

    let out = rebase(&current, &synced, false);
    assert!(out.slot_kinds().contains(&SlotKind::Hello));
    assert_eq!(titles(&out.payload), ["local", "synced"]);

    let mut rotated = synced.clone();
    rotated.rotate(&mut OsRng, PW, KDF).unwrap();
    let out = rebase(&current, &rotated, true);
    assert!(out.same_key(&rotated));
    assert!(!out.slot_kinds().contains(&SlotKind::Hello));
}

// ---------------------------------------------------------------------------
// Key-less push on lock / exit (S8)
// ---------------------------------------------------------------------------

#[test]
fn keyless_push_uploads_the_local_file_when_the_remote_did_not_move() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    a.lock();
    let file = a.file();
    let sha = block_on(push_raw(&store, &file, a.state().base_sha.as_deref()))
        .unwrap()
        .expect("pushed");
    a.inner().state = SyncState::synced(sha, &file);
    assert!(!a.dirty());
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(b.titles(), ["GitHub", "Mail"]);
}

#[test]
fn keyless_push_does_nothing_when_the_remote_moved() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    b.edit(|p| p.entries.push(entry(3, "Cloud", 20)));
    b.sync(&store).unwrap();
    a.edit(|p| p.entries.push(entry(2, "Mail", 20)));
    a.lock();
    let puts = store.puts();
    let pushed = block_on(push_raw(&store, &a.file(), a.state().base_sha.as_deref())).unwrap();
    assert_eq!(pushed, None);
    assert_eq!(store.puts(), puts);
    a.unlock(Unlock::Password(PW)).unwrap();
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(a.titles(), ["GitHub", "Mail", "Cloud"]);
}

// ---------------------------------------------------------------------------
// A remote that goes back to an older copy (v0.7 U10: history rewrite racing a push)
// ---------------------------------------------------------------------------

#[test]
fn remote_moved_back_after_a_concurrent_push_loses_no_edit() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    // The history rewrite reads the branch while it holds this copy ...
    let before = store.current().unwrap();
    // ... B pushes an edit ...
    b.edit(|p| p.entries.push(entry(2, "Bank", 20)));
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pushed);
    assert!(!b.dirty());
    // ... and the forced ref update puts the older copy back.
    store.force(&before.bytes);

    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::UpToDate);
    assert_eq!(
        b.sync(&store).unwrap(),
        SyncOutcome::Merged,
        "a clean device does not adopt a remote that lacks its edits"
    );
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(a.titles(), ["GitHub", "Bank"]);
    assert_eq!(b.titles(), ["GitHub", "Bank"]);
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::UpToDate);
}

#[test]
fn remote_moved_back_after_an_edit_and_a_delete_keeps_both() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    let before = store.current().unwrap();
    b.edit(|p| {
        p.entries[0].title = "GitHub (work)".into();
        p.entries[0].updated_at = NOW - 2_000;
        p.entries.push(entry(2, "Old mail", 20));
    });
    b.sync(&store).unwrap();
    b.edit(|p| p.entries[1].delete(NOW - 1_000));
    b.sync(&store).unwrap();
    store.force(&before.bytes);

    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Merged);
    assert_eq!(a.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(a.titles(), ["GitHub (work)", "~2"]);
    assert_eq!(a.titles(), b.titles());
}

#[test]
fn an_expired_tombstone_missing_on_the_remote_is_not_pushed_back() {
    let store = MemoryStore::new();
    let (a, b) = two_devices(&store);
    a.edit(|p| p.entries[0].delete(NOW - 1_000));
    a.sync(&store).unwrap();
    assert_eq!(b.sync(&store).unwrap(), SyncOutcome::Pulled);
    assert_eq!(b.titles(), ["~1"]);

    // Half a year later A pushes again; the push drops the expired tombstone.
    let later = NOW + TOMBSTONE_TTL_MS;
    a.edit(|p| p.entries.push(entry(2, "Mail", later)));
    assert_eq!(
        block_on(sync_once(&store, &mut a.clone(), later)).unwrap(),
        SyncOutcome::Pushed
    );
    assert_eq!(a.titles(), ["Mail"]);
    // B still has the tombstone: it adopts the remote instead of pushing it back.
    assert_eq!(
        block_on(sync_once(&store, &mut b.clone(), later)).unwrap(),
        SyncOutcome::Pulled
    );
    assert_eq!(b.titles(), ["Mail"]);
}
