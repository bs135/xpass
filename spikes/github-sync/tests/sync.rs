use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use sync_spike::{
    engine::{Device, SyncError, SyncOutcome},
    merge::merge,
    store::{blob_sha, MemoryStore},
};
use tempfile::TempDir;
use uuid::Uuid;
use vault_format::{
    crypto::KdfParams,
    model::{Entry, Payload},
    Error as VaultError, Unlock, UnlockedVault,
};

const KDF: KdfParams = KdfParams { m_kib: 8 * 1024, t: 1, p: 1 };
const PW: &[u8] = b"pw";

fn entry(id: u128, title: &str, updated_at: i64) -> Entry {
    let mut e = Entry::new(title, 1_000);
    e.id = Uuid::from_u128(id);
    e.updated_at = updated_at;
    e
}

fn titles(p: &Payload) -> Vec<String> {
    p.entries.iter().map(|e| if e.deleted { format!("~{}", e.id.as_u128()) } else { e.title.clone() }).collect()
}

/// Device A created + pushed, device B cloned. Both share `store`.
fn two_devices(store: &MemoryStore) -> (TempDir, Device<&MemoryStore>, Device<&MemoryStore>) {
    let tmp = TempDir::new().unwrap();
    let (mut a, _) = Device::create(&tmp.path().join("a"), store, PW, KDF).unwrap();
    a.edit(|p| p.entries.push(entry(1, "GitHub", 10))).unwrap();
    assert_eq!(a.sync().unwrap(), SyncOutcome::Pushed);
    let b = Device::clone_remote(&tmp.path().join("b"), store, PW).unwrap();
    (tmp, a, b)
}

// ---------------------------------------------------------------------------
// merge
// ---------------------------------------------------------------------------

#[test]
fn blob_sha_matches_git() {
    // `printf 'hello\n' | git hash-object --stdin`
    assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
}

#[test]
fn merge_rules() {
    let a = Payload { schema: 1, revision: 3, entries: vec![entry(1, "old", 10), entry(2, "only-a", 10), entry(3, "edit", 50)] };
    let mut tomb = entry(3, "x", 40);
    tomb.delete(40);
    let b = Payload { schema: 1, revision: 5, entries: vec![entry(1, "new", 20), entry(4, "only-b", 10), tomb] };
    let m = merge(&a, &b);
    assert_eq!(titles(&m), vec!["new", "only-a", "edit", "only-b"], "newer wins, union of ids, edit at 50 beats delete at 40");
    assert_eq!(m.revision, 5);

    // Same timestamp: delete wins.
    let mut del = entry(1, "", 20);
    del.delete(20);
    let m = merge(&b, &Payload { schema: 1, revision: 0, entries: vec![del] });
    assert!(m.entries.iter().find(|e| e.id == Uuid::from_u128(1)).unwrap().deleted);
}

#[test]
fn merge_converges_regardless_of_order() {
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    for _ in 0..300 {
        let mut gen = || Payload {
            schema: 1,
            revision: rng.gen_range(0..10),
            entries: (0..rng.gen_range(0..8))
                .map(|_| {
                    let mut e = entry(rng.gen_range(0..6), &format!("t{}", rng.gen_range(0..3)), rng.gen_range(0..4));
                    if rng.gen_bool(0.3) {
                        let t = e.updated_at;
                        e.delete(t);
                    }
                    e
                })
                // one copy per id within a single payload
                .fold(Vec::<Entry>::new(), |mut v, e| {
                    if !v.iter().any(|x| x.id == e.id) {
                        v.push(e);
                    }
                    v
                }),
        };
        let (x, y, z) = (gen(), gen(), gen());
        assert_eq!(merge(&x, &y), merge(&y, &x), "commutative");
        assert_eq!(merge(&x, &x).entries, x.entries.iter().cloned().collect::<Vec<_>>().tap_sort(), "idempotent");
        assert_eq!(merge(&merge(&x, &y), &z), merge(&x, &merge(&y, &z)), "associative");
    }
}

trait TapSort {
    fn tap_sort(self) -> Self;
}
impl TapSort for Vec<Entry> {
    fn tap_sort(mut self) -> Self {
        self.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        self
    }
}

// ---------------------------------------------------------------------------
// sync engine
// ---------------------------------------------------------------------------

#[test]
fn second_sync_is_a_no_op() {
    let store = MemoryStore::new();
    let (_t, mut a, mut b) = two_devices(&store);
    assert_eq!(a.sync().unwrap(), SyncOutcome::UpToDate);
    assert_eq!(b.sync().unwrap(), SyncOutcome::UpToDate);
    assert_eq!(*store.puts.lock().unwrap(), 1);
}

#[test]
fn disjoint_edits_merge() {
    let store = MemoryStore::new();
    let (_t, mut a, mut b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "from A", 20))).unwrap();
    b.edit(|p| p.entries.push(entry(3, "from B", 20))).unwrap();

    assert_eq!(a.sync().unwrap(), SyncOutcome::Pushed);
    assert_eq!(b.sync().unwrap(), SyncOutcome::Merged);
    assert_eq!(a.sync().unwrap(), SyncOutcome::Pulled);
    assert_eq!(titles(a.payload()), vec!["GitHub", "from A", "from B"]);
    assert_eq!(titles(a.payload()), titles(b.payload()));
}

#[test]
fn same_entry_newer_edit_wins_on_both_devices() {
    let store = MemoryStore::new();
    let (_t, mut a, mut b) = two_devices(&store);
    a.edit(|p| p.entries[0] = entry(1, "A edit", 30)).unwrap();
    b.edit(|p| p.entries[0] = entry(1, "B edit (newer)", 40)).unwrap();
    // B syncs first this time: order must not matter.
    assert_eq!(b.sync().unwrap(), SyncOutcome::Pushed);
    assert_eq!(a.sync().unwrap(), SyncOutcome::Merged);
    b.sync().unwrap();
    assert_eq!(titles(a.payload()), vec!["B edit (newer)"]);
    assert_eq!(titles(b.payload()), vec!["B edit (newer)"]);
}

#[test]
fn delete_is_not_resurrected_by_stale_device() {
    let store = MemoryStore::new();
    let (_t, mut a, mut b) = two_devices(&store);
    a.edit(|p| p.entries[0].delete(50)).unwrap();
    a.sync().unwrap();
    // B never saw the delete and edits something else.
    b.edit(|p| p.entries.push(entry(2, "other", 60))).unwrap();
    assert_eq!(b.sync().unwrap(), SyncOutcome::Merged);
    assert_eq!(titles(b.payload()), vec!["~1", "other"]);
}

#[test]
fn race_between_get_and_put_is_retried() {
    let store = MemoryStore::new();
    let (_t, mut a, mut b) = two_devices(&store);
    a.edit(|p| p.entries.push(entry(2, "from A", 20))).unwrap();
    b.edit(|p| p.entries.push(entry(3, "from B", 20))).unwrap();

    // A pushes first. Then device C pushes *between* B's GET and B's PUT.
    a.sync().unwrap();
    let mut c = UnlockedVault::unlock(&store.current().unwrap().bytes, Unlock::Password(PW)).unwrap();
    c.payload.entries.push(entry(4, "late push from C", 25));
    let late = c.save(&mut rand::rngs::OsRng).unwrap();
    store.before_next_put(move |s| s.force(&late));

    let puts_before = *store.puts.lock().unwrap();
    assert_eq!(b.sync().unwrap(), SyncOutcome::Merged, "first put conflicts, retry merges");
    assert_eq!(*store.puts.lock().unwrap(), puts_before + 1, "exactly one successful put after the conflict");
    assert_eq!(titles(b.payload()), vec!["GitHub", "from A", "from B", "late push from C"]);
}

#[test]
fn offline_edits_are_kept_and_pushed_later() {
    let store = MemoryStore::new();
    let (t, mut a, _b) = two_devices(&store);
    store.set_offline(true);
    a.edit(|p| p.entries.push(entry(2, "offline edit", 20))).unwrap();
    assert_eq!(a.sync().unwrap(), SyncOutcome::Offline);
    assert!(a.is_dirty());

    // App restarts while still offline: local copy opens without network.
    drop(a);
    let mut a = Device::open(&t.path().join("a"), &store, PW).unwrap();
    assert!(a.is_dirty());
    assert_eq!(titles(a.payload()), vec!["GitHub", "offline edit"]);

    store.set_offline(false);
    assert_eq!(a.sync().unwrap(), SyncOutcome::Pushed);
    assert!(!a.is_dirty());
    let remote = UnlockedVault::unlock(&store.current().unwrap().bytes, Unlock::Password(PW)).unwrap();
    assert_eq!(titles(&remote.payload), vec!["GitHub", "offline edit"]);
}

#[test]
fn password_change_on_other_device_is_adopted() {
    let store = MemoryStore::new();
    let (t, mut a, mut b) = two_devices(&store);
    // A changes the password (same VaultKey) and pushes.
    let mut v = UnlockedVault::unlock(&store.current().unwrap().bytes, Unlock::Password(PW)).unwrap();
    v.change_password(&mut rand::rngs::OsRng, b"new pw", KDF).unwrap();
    store.force(&v.save(&mut rand::rngs::OsRng).unwrap());

    // B syncs with its in-memory key, no prompt, and adopts the new slots.
    b.edit(|p| p.entries.push(entry(2, "from B", 20))).unwrap();
    assert_eq!(b.sync().unwrap(), SyncOutcome::Merged);
    drop(b);
    assert!(Device::open(&t.path().join("b"), &store, PW).is_err(), "old password no longer opens B's local copy");
    Device::open(&t.path().join("b"), &store, b"new pw").unwrap();
    let _ = a.sync();
}

#[test]
fn rotation_on_other_device_requires_credentials() {
    let store = MemoryStore::new();
    let (_t, _a, mut b) = two_devices(&store);
    let mut v = UnlockedVault::unlock(&store.current().unwrap().bytes, Unlock::Password(PW)).unwrap();
    v.rotate(&mut rand::rngs::OsRng, b"rotated", KDF).unwrap();
    store.force(&v.save(&mut rand::rngs::OsRng).unwrap());

    b.edit(|p| p.entries.push(entry(2, "from B", 20))).unwrap();
    match b.sync() {
        Err(SyncError::Vault(VaultError::KeyChanged)) => {}
        other => panic!("expected KeyChanged, got {other:?}"),
    }
    assert!(b.is_dirty(), "local edit kept until the user re-authenticates");
}

#[test]
fn remote_from_another_vault_is_refused() {
    let store = MemoryStore::new();
    let (_t, _a, mut b) = two_devices(&store);
    let (mut other, _) = UnlockedVault::create(&mut rand::rngs::OsRng, PW, KDF).unwrap();
    store.force(&other.save(&mut rand::rngs::OsRng).unwrap());
    b.edit(|p| p.entries.push(entry(2, "x", 20))).unwrap();
    assert!(matches!(b.sync(), Err(SyncError::Vault(VaultError::DifferentVault))));
}
