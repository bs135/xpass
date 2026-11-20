//! Writes imported rows into a payload (plan I5, I6).
//!
//! The caller runs this inside one vault edit, so the whole import gets one D2
//! timestamp `ts`, one atomic write and one sync push. New entries get fresh ids, so
//! other devices just receive them; updated entries merge by LWW like any edit.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xpass_core::model::{Entry, HistoryItem, Payload, HISTORY_NOTE_IMPORT};

use crate::{
    convert::ImportRow,
    dedup::{classify, Duplicate, Match},
};

/// What to do with possible duplicates (same site and username, different password).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateChoice {
    #[default]
    Skip,
    /// Import as a separate entry.
    New,
    /// Replace the matched entry's password; the old one goes to its history.
    Update,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    #[serde(default)]
    pub duplicates: DuplicateChoice,
    /// Row indices the user unticked.
    #[serde(default)]
    pub excluded: BTreeSet<usize>,
}

/// Counts over all rows: `added + updated + skipped == rows.len()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Adds or updates entries for `rows`, all stamped with `ts`. Exact duplicates and
/// excluded rows are skipped; possible duplicates follow `options.duplicates`.
pub fn apply_import(payload: &mut Payload, rows: &[ImportRow], options: &ImportOptions, ts: i64) -> ImportSummary {
    let plan = classify(payload, rows, &options.excluded);
    let mut summary = ImportSummary::default();
    // Entry each row ended up in (added, updated or matched exactly), for later rows
    // of the same file that update it.
    let mut target: Vec<Option<Uuid>> = vec![None; rows.len()];

    for (index, (row, dup)) in rows.iter().zip(plan).enumerate() {
        if options.excluded.contains(&index) {
            summary.skipped += 1;
            continue;
        }
        match dup {
            Duplicate::Exact { of } => {
                target[index] = resolve(of, &target);
                summary.skipped += 1;
            }
            Duplicate::Possible { of } if options.duplicates == DuplicateChoice::Update => {
                match resolve(of, &target).and_then(|id| update_password(payload, id, row, ts)) {
                    Some(id) => {
                        target[index] = Some(id);
                        summary.updated += 1;
                    }
                    None => {
                        target[index] = Some(add(payload, row, ts));
                        summary.added += 1;
                    }
                }
            }
            Duplicate::Possible { .. } if options.duplicates == DuplicateChoice::Skip => {
                summary.skipped += 1;
            }
            Duplicate::Possible { .. } | Duplicate::None => {
                target[index] = Some(add(payload, row, ts));
                summary.added += 1;
            }
        }
    }
    summary
}

fn resolve(of: Match, target: &[Option<Uuid>]) -> Option<Uuid> {
    match of {
        Match::Entry { id } => Some(id),
        Match::Row { index } => target[index],
    }
}

fn add(payload: &mut Payload, row: &ImportRow, ts: i64) -> Uuid {
    let mut e = Entry::new(&row.title, ts);
    // A source creation date is kept unless it is in the future (plan I6).
    e.created_at = row.created_at.filter(|c| (0..=ts).contains(c)).unwrap_or(ts);
    e.username.clone_from(&row.username);
    e.password.clone_from(&row.password);
    e.urls.clone_from(&row.urls);
    e.notes.clone_from(&row.notes);
    e.tags.clone_from(&row.tags);
    e.favorite = row.favorite;
    e.fields.clone_from(&row.fields);
    let id = e.id;
    payload.entries.push(e);
    id
}

/// Moves the entry's current credentials to its history (note `import`) and sets the
/// row's password. `None` when the entry is gone.
fn update_password(payload: &mut Payload, id: Uuid, row: &ImportRow, ts: i64) -> Option<Uuid> {
    let e = payload.entries.iter_mut().find(|e| e.id == id && !e.deleted)?;
    if e.password != row.password {
        if !e.password.is_empty() {
            let mut old = HistoryItem::credentials_of(e, Some(HISTORY_NOTE_IMPORT));
            old.at = ts;
            e.push_history(old);
        }
        e.password.clone_from(&row.password);
        e.updated_at = ts;
        e.normalize_history();
    }
    Some(id)
}
