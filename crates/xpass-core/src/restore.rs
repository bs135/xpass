//! Restoring a previous version (a local backup) as an ordinary edit (S14).
//!
//! Simply putting an old payload back would lose every LWW comparison against the
//! newer copies on other devices, so the next sync would undo the restore. Instead
//! every entry that differs is rewritten with a fresh timestamp:
//! - an entry that changed since the backup gets the backup's content, stamped `ts`;
//!   the replaced credentials go to its history (note [`HISTORY_NOTE_RESTORE`]);
//! - an entry deleted since the backup comes back, stamped `ts`;
//! - an entry created since the backup becomes a tombstone at `ts`;
//! - identical entries are left alone (no new timestamp, nothing to sync).
//!
//! `ts` must be newer than everything in both payloads (`clock::next_timestamp`).

use std::collections::BTreeMap;

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::model::{normalized_history, Entry, HistoryItem, Payload, HISTORY_NOTE_RESTORE};

pub fn restore(current: &Payload, backup: &Payload, ts: i64) -> Payload {
    let old: BTreeMap<Uuid, &Entry> = backup.entries.iter().map(|e| (e.id, e)).collect();
    let mut entries: Vec<Entry> = Vec::with_capacity(current.entries.len());
    for c in &current.entries {
        let next = match old.get(&c.id) {
            Some(b) if content(c) == content(b) => c.clone(),
            Some(b) => restored(c, b, ts),
            None if c.deleted => c.clone(),
            None => {
                let mut gone = c.clone();
                gone.delete(ts);
                gone
            }
        };
        entries.push(next);
    }
    let known: std::collections::BTreeSet<Uuid> = current.entries.iter().map(|e| e.id).collect();
    for b in backup.entries.iter().filter(|b| !b.deleted && !known.contains(&b.id)) {
        let mut back = b.clone();
        back.updated_at = ts;
        entries.push(back);
    }
    entries.sort_by(|x, y| x.created_at.cmp(&y.created_at).then(x.id.cmp(&y.id)));
    Payload {
        schema: current.schema.max(backup.schema),
        revision: current.revision,
        entries,
        extra: current.extra.clone(),
    }
}

/// The backup's version of `current`, stamped `ts`.
fn restored(current: &Entry, backup: &Entry, ts: i64) -> Entry {
    if backup.deleted {
        let mut gone = current.clone();
        gone.delete(ts);
        return gone;
    }
    let mut out = backup.clone();
    out.updated_at = ts;
    let replaced = (!current.deleted).then(|| HistoryItem::credentials_of(current, Some(HISTORY_NOTE_RESTORE)));
    let items = current.history.iter().chain(&backup.history).chain(replaced.as_ref());
    out.history = normalized_history(items, &out);
    out
}

/// Entry content without the fields a restore or merge rewrites.
fn content(e: &Entry) -> Zeroizing<Vec<u8>> {
    let mut e = e.clone();
    e.history.clear();
    e.updated_at = 0;
    Zeroizing::new(serde_json::to_vec(&e).expect("Entry serializes to JSON"))
}
