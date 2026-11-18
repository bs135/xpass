use uuid::Uuid;
use xpass_core::{
    merge::merge,
    model::{Entry, Extra, Payload, HISTORY_NOTE_RESTORE},
    restore::restore,
};

fn entry(id: u128, title: &str, password: &str, updated_at: i64) -> Entry {
    let mut e = Entry::new(title, 1_000 + id as i64);
    e.id = Uuid::from_u128(id);
    e.password = password.into();
    e.updated_at = updated_at;
    e
}

fn payload(entries: Vec<Entry>) -> Payload {
    Payload {
        schema: 1,
        revision: 7,
        entries,
        extra: Extra::new(),
    }
}

fn find(p: &Payload, id: u128) -> &Entry {
    p.entries
        .iter()
        .find(|e| e.id == Uuid::from_u128(id))
        .expect("entry present")
}

/// Backup at t=100: entries 1 (unchanged later), 2 (edited later), 3 (deleted later).
/// Current at t=500: 1 same, 2 edited, 3 tombstone, 4 created after the backup.
fn scenario() -> (Payload, Payload) {
    let backup = payload(vec![
        entry(1, "Same", "s", 100),
        entry(2, "Old title", "old-pw", 100),
        entry(3, "Deleted later", "d", 100),
    ]);
    let mut gone = entry(3, "Deleted later", "d", 100);
    gone.delete(400);
    let current = payload(vec![
        entry(1, "Same", "s", 100),
        entry(2, "New title", "new-pw", 300),
        gone,
        entry(4, "Created later", "c", 450),
    ]);
    (current, backup)
}

#[test]
fn restore_rewrites_only_what_differs_with_a_new_timestamp() {
    let (current, backup) = scenario();
    let out = restore(&current, &backup, 1_000);

    assert_eq!(find(&out, 1), find(&current, 1), "unchanged entry left alone");

    let e2 = find(&out, 2);
    assert_eq!(
        (e2.title.as_str(), e2.password.as_str(), e2.updated_at),
        ("Old title", "old-pw", 1_000)
    );
    let replaced = e2
        .history
        .iter()
        .find(|h| h.password == "new-pw")
        .expect("replaced credentials kept");
    assert_eq!(replaced.note.as_deref(), Some(HISTORY_NOTE_RESTORE));

    let e3 = find(&out, 3);
    assert!(!e3.deleted, "deleted entry comes back");
    assert_eq!((e3.title.as_str(), e3.updated_at), ("Deleted later", 1_000));

    let e4 = find(&out, 4);
    assert!(e4.deleted, "entry created after the backup is deleted");
    assert_eq!(e4.updated_at, 1_000);

    assert_eq!(out.revision, current.revision);
    assert_eq!(out.entries.len(), 4);
}

#[test]
fn restored_state_wins_the_merge_against_other_devices() {
    let (current, backup) = scenario();
    let restored = restore(&current, &backup, 1_000);
    // Another device still has the current state (older timestamps).
    let merged = merge(&current, &restored);
    for id in 1..=4 {
        let (a, b) = (find(&merged, id), find(&restored, id));
        assert_eq!(
            (a.title.as_str(), a.deleted, a.updated_at),
            (b.title.as_str(), b.deleted, b.updated_at)
        );
    }
}

#[test]
fn entry_only_in_the_backup_is_brought_back() {
    let backup = payload(vec![entry(5, "Purged tombstone", "p", 100)]);
    let current = payload(vec![]);
    let out = restore(&current, &backup, 1_000);
    assert_eq!(out.entries.len(), 1);
    assert_eq!(out.entries[0].updated_at, 1_000);
}

#[test]
fn restoring_the_current_state_changes_nothing() {
    let (current, _) = scenario();
    assert_eq!(restore(&current, &current, 1_000), current);
}
