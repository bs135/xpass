//! Per-entry last-writer-wins merge with conflict history (D1).
//!
//! Rules for an id present on both sides:
//! 1. Higher `updated_at` wins.
//! 2. Tie: the tombstone wins (a delete is never undone by a same-time edit).
//! 3. Still tied: compare the serialized entries (without `history`) and keep the
//!    larger one, so every device picks the same winner.
//!
//! D1: the winner keeps its content, and the credentials of every losing version
//! (`password`, `username`) are added to the winner's `history` with
//! `note = "conflict"` and `at = loser.updated_at`. Histories of all versions are
//! united and brought into canonical form ([`normalized_history`]). A tombstone
//! winner keeps an empty history: deletes wipe secrets.
//!
//! Properties (tested):
//! - commutative and idempotent, always;
//! - associative for winner selection, always; for history only while no tombstone
//!   sits between versions and histories stay under [`HISTORY_LIMIT`]
//!   (a version that lost to a tombstone may or may not reach a later re-edit's
//!   history, depending on sync order). Devices still converge because every sync
//!   goes through the single remote file.
//!
//! [`HISTORY_LIMIT`]: crate::model::HISTORY_LIMIT

use std::{cmp::Ordering, collections::BTreeMap};

use serde_json::Value;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::model::{normalized_history, Entry, Extra, HistoryItem, Payload, HISTORY_NOTE_CONFLICT};

pub fn merge(a: &Payload, b: &Payload) -> Payload {
    let mut by_id: BTreeMap<Uuid, Vec<&Entry>> = BTreeMap::new();
    for e in a.entries.iter().chain(&b.entries) {
        by_id.entry(e.id).or_default().push(e);
    }
    let mut entries: Vec<Entry> = by_id.into_values().map(|versions| merge_versions(&versions)).collect();
    entries.sort_by(|x, y| x.created_at.cmp(&y.created_at).then(x.id.cmp(&y.id)));
    Payload {
        schema: a.schema.max(b.schema),
        revision: a.revision.max(b.revision),
        entries,
        extra: merge_extra(&a.extra, &b.extra),
    }
}

/// `theirs` already holds every entry of `ours` at the same or a winning version, so
/// `merge(ours, theirs)` would bring nothing new to `theirs`. Tombstones of `ours`
/// older than `purge_before` are ignored: a push drops them anyway (D3).
///
/// A clean device adopts the remote only when this holds; otherwise the remote went
/// back to an older copy (a history rewrite racing a push, or a rolled-back file, A8)
/// and the device merges and pushes instead of losing its own edits.
pub fn covers(theirs: &Payload, ours: &Payload, purge_before: i64) -> bool {
    let by_id: BTreeMap<Uuid, &Entry> = theirs.entries.iter().map(|e| (e.id, e)).collect();
    ours.entries
        .iter()
        .filter(|o| !(o.deleted && o.updated_at < purge_before))
        .all(|o| by_id.get(&o.id).is_some_and(|t| lww_cmp(t, o) != Ordering::Less))
}

/// Merges all versions of one entry id (non-empty).
fn merge_versions(versions: &[&Entry]) -> Entry {
    let winner = *versions
        .iter()
        .max_by(|x, y| lww_cmp(x, y))
        .expect("at least one version");
    let mut out = winner.clone();
    if winner.deleted {
        return out;
    }
    // The winner's own conflict item is dropped by normalization (it matches the
    // current credentials); adding it keeps the result independent of which side won.
    let conflicts: Vec<HistoryItem> = versions
        .iter()
        .filter(|v| !v.deleted)
        .map(|v| conflict_item(v))
        .collect();
    let all = versions.iter().flat_map(|v| v.history.iter()).chain(&conflicts);
    out.history = normalized_history(all, winner);
    out
}

/// Total order used to pick the winner.
fn lww_cmp(x: &Entry, y: &Entry) -> Ordering {
    x.updated_at
        .cmp(&y.updated_at)
        .then(x.deleted.cmp(&y.deleted))
        .then_with(|| content_bytes(x).cmp(&content_bytes(y)))
}

/// Canonical bytes of an entry without its history (history differs between
/// intermediate merge results and must not affect the winner).
fn content_bytes(e: &Entry) -> Zeroizing<Vec<u8>> {
    let mut e = e.clone();
    e.history.clear();
    Zeroizing::new(serde_json::to_vec(&e).expect("Entry serializes to JSON"))
}

fn conflict_item(loser: &Entry) -> HistoryItem {
    HistoryItem::credentials_of(loser, Some(HISTORY_NOTE_CONFLICT))
}

/// Unknown payload fields: union; on a key conflict the larger serialized value wins.
fn merge_extra(a: &Extra, b: &Extra) -> Extra {
    let mut out = a.clone();
    for (k, v) in b {
        match out.get(k) {
            Some(cur) if json_bytes(cur) >= json_bytes(v) => {}
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

fn json_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).expect("Value serializes to JSON")
}
