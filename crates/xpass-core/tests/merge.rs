use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use uuid::Uuid;
use xpass_core::{
    clock::next_timestamp,
    maintenance::{purge_tombstones, TOMBSTONE_TTL_MS},
    merge::merge,
    model::{Entry, Extra, HistoryItem, Payload, HISTORY_LIMIT, HISTORY_NOTE_CONFLICT},
};

fn entry(id: u128, title: &str, updated_at: i64) -> Entry {
    let mut e = Entry::new(title, 1_000);
    e.id = Uuid::from_u128(id);
    e.updated_at = updated_at;
    e
}

fn creds(id: u128, password: &str, username: &str, updated_at: i64) -> Entry {
    let mut e = entry(id, "T", updated_at);
    e.password = password.into();
    e.username = username.into();
    e
}

fn payload(revision: u64, entries: Vec<Entry>) -> Payload {
    Payload {
        schema: 1,
        revision,
        entries,
        extra: Extra::new(),
    }
}

fn tombstone(id: u128, at: i64) -> Entry {
    let mut e = entry(id, "x", at);
    e.delete(at);
    e
}

fn hist(at: i64, password: &str) -> HistoryItem {
    HistoryItem {
        at,
        password: password.into(),
        username: None,
        note: None,
    }
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

fn find(p: &Payload, id: u128) -> &Entry {
    p.entries.iter().find(|e| e.id == Uuid::from_u128(id)).unwrap()
}

// ---- LWW rules (ported) ----

#[test]
fn merge_rules() {
    let a = payload(
        3,
        vec![entry(1, "old", 10), entry(2, "only-a", 10), entry(3, "edit", 50)],
    );
    let b = payload(5, vec![entry(1, "new", 20), entry(4, "only-b", 10), tombstone(3, 40)]);
    let m = merge(&a, &b);
    assert_eq!(
        titles(&m),
        vec!["new", "only-a", "edit", "only-b"],
        "newer wins, union of ids, edit at 50 beats delete at 40"
    );
    assert_eq!(m.revision, 5);

    // Same timestamp: delete wins.
    let m = merge(&b, &payload(0, vec![tombstone(1, 20)]));
    assert!(find(&m, 1).deleted);
}

// ---- D1: losing credentials go to history ----

#[test]
fn loser_credentials_are_kept_in_history() {
    let a = payload(1, vec![creds(1, "pw-a", "alice", 10)]);
    let b = payload(1, vec![creds(1, "pw-b", "alice2", 20)]);
    let m = merge(&a, &b);
    let e = find(&m, 1);
    assert_eq!((e.password.as_str(), e.username.as_str()), ("pw-b", "alice2"));
    assert_eq!(
        e.history,
        vec![HistoryItem {
            at: 10,
            password: "pw-a".into(),
            username: Some("alice".into()),
            note: Some(HISTORY_NOTE_CONFLICT.into()),
        }]
    );
}

#[test]
fn same_credentials_add_no_history() {
    let mut a = creds(1, "pw", "alice", 10);
    a.title = "Old title".into();
    let mut b = creds(1, "pw", "alice", 20);
    b.title = "New title".into();
    let m = merge(&payload(1, vec![a]), &payload(1, vec![b]));
    assert!(
        find(&m, 1).history.is_empty(),
        "a title-only conflict leaves no history noise"
    );
}

#[test]
fn histories_are_united_and_deduplicated() {
    let mut a = creds(1, "pw-a", "", 10);
    a.history = vec![hist(1, "old-1"), hist(2, "old-2")];
    let mut b = creds(1, "pw-b", "", 20);
    b.history = vec![hist(2, "old-2"), hist(3, "old-3")];
    let m = merge(&payload(1, vec![a]), &payload(1, vec![b]));
    let pw: Vec<_> = find(&m, 1).history.iter().map(|h| h.password.as_str()).collect();
    assert_eq!(
        pw,
        vec!["old-1", "old-2", "old-3", "pw-a"],
        "sorted by at, duplicates removed"
    );
}

#[test]
fn history_never_contains_the_current_password() {
    // The winner reverted to a password that the loser has in its history.
    let mut a = creds(1, "pw-1", "", 10);
    a.history = vec![hist(5, "pw-0")];
    let b = creds(1, "pw-0", "", 20);
    let m = merge(&payload(1, vec![a]), &payload(1, vec![b]));
    let pw: Vec<_> = find(&m, 1).history.iter().map(|h| h.password.as_str()).collect();
    assert_eq!(pw, vec!["pw-1"]);
}

#[test]
fn merged_history_is_capped_to_the_newest() {
    let mut a = creds(1, "pw-a", "", 1_000);
    a.history = (0..HISTORY_LIMIT as i64).map(|i| hist(i, &format!("a{i}"))).collect();
    let mut b = creds(1, "pw-b", "", 2_000);
    b.history = (0..HISTORY_LIMIT as i64)
        .map(|i| hist(100 + i, &format!("b{i}")))
        .collect();
    let m = merge(&payload(1, vec![a]), &payload(1, vec![b]));
    let h = &find(&m, 1).history;
    assert_eq!(h.len(), HISTORY_LIMIT);
    assert_eq!(h.last().unwrap().password, "pw-a", "the conflict item is the newest");
    assert!(h.iter().all(|i| !i.password.starts_with('a')), "oldest items dropped");
}

#[test]
fn tombstone_winner_keeps_no_history() {
    let mut a = creds(1, "pw", "alice", 10);
    a.history = vec![hist(1, "old")];
    let m = merge(&payload(1, vec![a]), &payload(1, vec![tombstone(1, 20)]));
    let e = find(&m, 1);
    assert!(e.deleted && e.history.is_empty() && e.password.is_empty());
}

#[test]
fn unknown_payload_fields_are_united() {
    let mut a = payload(1, vec![]);
    a.extra.insert("only_a".into(), "x".into());
    a.extra.insert("both".into(), 1.into());
    let mut b = payload(1, vec![]);
    b.extra.insert("only_b".into(), "y".into());
    b.extra.insert("both".into(), 2.into());
    let m = merge(&a, &b);
    assert_eq!(m.extra, merge(&b, &a).extra);
    assert_eq!(
        (m.extra["only_a"].as_str(), m.extra["only_b"].as_str()),
        (Some("x"), Some("y"))
    );
    assert_eq!(m.extra["both"], 2);
}

// ---- Algebra ----

fn random_payload(rng: &mut ChaCha20Rng, allow_tombstones: bool) -> Payload {
    let mut entries: Vec<Entry> = Vec::new();
    for _ in 0..rng.gen_range(0..8) {
        let id = rng.gen_range(0..6);
        if entries.iter().any(|e| e.id == Uuid::from_u128(id)) {
            continue; // one copy per id within a single payload
        }
        let updated_at = rng.gen_range(0..4);
        let mut e = creds(
            id,
            &format!("pw{}", rng.gen_range(0..3)),
            &format!("u{}", rng.gen_range(0..2)),
            updated_at,
        );
        e.title = format!("t{}", rng.gen_range(0..3));
        // History items are never newer than the entry itself.
        e.history = (0..rng.gen_range(0..3))
            .map(|_| hist(rng.gen_range(0..=updated_at), &format!("pw{}", rng.gen_range(0..4))))
            .collect();
        e.normalize_history();
        if allow_tombstones && rng.gen_bool(0.3) {
            e.delete(updated_at);
        }
        entries.push(e);
    }
    payload(rng.gen_range(0..10), entries)
}

fn sorted(p: &Payload) -> Vec<Entry> {
    let mut v = p.entries.clone();
    v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    v
}

fn without_history(p: Payload) -> Payload {
    let mut p = p;
    p.entries.iter_mut().for_each(|e| e.history.clear());
    p
}

#[test]
fn merge_is_commutative_and_idempotent() {
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    for _ in 0..500 {
        let (x, y) = (random_payload(&mut rng, true), random_payload(&mut rng, true));
        assert_eq!(merge(&x, &y), merge(&y, &x), "commutative");
        assert_eq!(merge(&x, &x).entries, sorted(&x), "idempotent");
        let m = merge(&x, &y);
        assert_eq!(merge(&m, &m), m, "idempotent on merge results");
        assert_eq!(merge(&m, &y), m, "re-merging a side changes nothing");
    }
}

#[test]
fn merge_is_associative() {
    let mut rng = ChaCha20Rng::seed_from_u64(8);
    for _ in 0..500 {
        let (x, y, z) = (
            random_payload(&mut rng, false),
            random_payload(&mut rng, false),
            random_payload(&mut rng, false),
        );
        assert_eq!(
            merge(&merge(&x, &y), &z),
            merge(&x, &merge(&y, &z)),
            "associative incl. history"
        );
    }
}

#[test]
fn winner_selection_is_associative_with_tombstones() {
    let mut rng = ChaCha20Rng::seed_from_u64(9);
    for _ in 0..500 {
        let (x, y, z) = (
            random_payload(&mut rng, true),
            random_payload(&mut rng, true),
            random_payload(&mut rng, true),
        );
        assert_eq!(
            without_history(merge(&merge(&x, &y), &z)),
            without_history(merge(&x, &merge(&y, &z))),
            "same winners regardless of order"
        );
    }
}

// ---- D2: clock ----

#[test]
fn slow_clock_still_wins_after_seeing_newer_edits() {
    let remote = payload(1, vec![creds(1, "from-fast-clock", "", 5_000)]);
    // This device's wall clock says 1_000 but it has seen 5_000.
    let ts = next_timestamp(1_000, remote.max_updated_at());
    let local = payload(1, vec![creds(1, "edited-here", "", ts)]);
    assert_eq!(find(&merge(&remote, &local), 1).password, "edited-here");
    assert_eq!(payload(0, vec![]).max_updated_at(), 0);
}

// ---- D3: maintenance ----

#[test]
fn old_tombstones_are_purged() {
    let now = 1_000 * TOMBSTONE_TTL_MS;
    let mut p = payload(
        1,
        vec![
            tombstone(1, now - TOMBSTONE_TTL_MS - 1),
            tombstone(2, now - TOMBSTONE_TTL_MS),
            entry(3, "alive and old", 0),
            tombstone(4, now),
        ],
    );
    assert_eq!(purge_tombstones(&mut p, now, TOMBSTONE_TTL_MS), 1);
    assert_eq!(titles(&p), vec!["~2", "alive and old", "~4"]);
    assert_eq!(purge_tombstones(&mut p, now, TOMBSTONE_TTL_MS), 0);
}
