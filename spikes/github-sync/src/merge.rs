//! Per-entry last-writer-wins merge.
//!
//! Properties (tested): commutative, idempotent, associative on entries, so all
//! devices converge to the same set regardless of sync order.
//!
//! Rules for an id present on both sides:
//! 1. Higher `updated_at` wins.
//! 2. Tie: the tombstone wins (a delete is never undone by a same-time edit).
//! 3. Still tied: compare the serialized entries and keep the larger one, so
//!    every device picks the same winner.

use std::{cmp::Ordering, collections::BTreeMap};

use uuid::Uuid;
use vault_format::model::{Entry, Payload};

pub fn merge(a: &Payload, b: &Payload) -> Payload {
    let mut by_id: BTreeMap<Uuid, &Entry> = BTreeMap::new();
    for e in a.entries.iter().chain(b.entries.iter()) {
        by_id
            .entry(e.id)
            .and_modify(|cur| {
                if wins(e, cur) {
                    *cur = e;
                }
            })
            .or_insert(e);
    }
    let mut entries: Vec<Entry> = by_id.into_values().cloned().collect();
    entries.sort_by(|x, y| x.created_at.cmp(&y.created_at).then(x.id.cmp(&y.id)));
    Payload { schema: a.schema.max(b.schema), revision: a.revision.max(b.revision), entries }
}

fn wins(challenger: &Entry, current: &Entry) -> bool {
    match challenger.updated_at.cmp(&current.updated_at) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => match (challenger.deleted, current.deleted) {
            (true, false) => true,
            (false, true) => false,
            _ => serde_json::to_vec(challenger).unwrap() > serde_json::to_vec(current).unwrap(),
        },
    }
}
