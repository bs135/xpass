//! Decrypted payload (schema 1, PLAN.md §3.4). Serialized as JSON inside the ciphertext.
//!
//! Merge-ready from day one: every entry has a stable `id` and `updated_at`, and
//! deletions are tombstones (`deleted = true`, content wiped) so another device's
//! copy cannot resurrect them.
//!
//! Forward compatibility (D5): fields this build does not know are kept in `extra`
//! (`#[serde(flatten)]`) and written back on save, so an older build never drops data
//! written by a newer one. **`extra` is not zeroized** on drop (`serde_json::Value`
//! does not support it); only unknown future fields live there.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SCHEMA_VERSION: u32 = 1;

/// Maximum number of [`HistoryItem`]s kept per entry; the oldest are dropped first.
pub const HISTORY_LIMIT: usize = 20;

/// `HistoryItem::note` for a version that lost a merge conflict (D1).
pub const HISTORY_NOTE_CONFLICT: &str = "conflict";

/// `HistoryItem::note` for the version replaced by restoring a backup.
pub const HISTORY_NOTE_RESTORE: &str = "restore";

/// `HistoryItem::note` for the credentials replaced by a CSV import ("update existing").
pub const HISTORY_NOTE_IMPORT: &str = "import";

/// Unknown JSON fields preserved across load/save (D5).
pub type Extra = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Payload {
    pub schema: u32,
    /// Incremented on every save; lets sync detect a stale overwrite.
    pub revision: u64,
    pub entries: Vec<Entry>,
    #[serde(flatten)]
    #[zeroize(skip)]
    pub extra: Extra,
}

impl Payload {
    /// Highest `updated_at` of any entry (tombstones included), or 0 when empty.
    /// Seeds the D2 clock (`clock::next_timestamp`) after unlock or merge.
    pub fn max_updated_at(&self) -> i64 {
        self.entries.iter().map(|e| e.updated_at).max().unwrap_or(0)
    }
}

impl Default for Payload {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            revision: 0,
            entries: Vec::new(),
            extra: Extra::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Entry {
    #[zeroize(skip)]
    pub id: Uuid,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    /// Stored since schema 1; edited in the UI in a later version.
    #[serde(default)]
    pub fields: Vec<CustomField>,
    /// Previous credentials, oldest first, at most [`HISTORY_LIMIT`].
    #[serde(default)]
    pub history: Vec<HistoryItem>,
    #[serde(flatten)]
    #[zeroize(skip)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct CustomField {
    pub name: String,
    pub value: String,
    /// Secret fields are treated like the password (hidden, never sent to the UI unasked).
    #[serde(default)]
    pub secret: bool,
}

/// Field order matters: the derived `Ord` (at, password, username, note) is the
/// canonical history order used by [`Entry::normalize_history`] and the merge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct HistoryItem {
    /// Unix milliseconds when this value was replaced.
    pub at: i64,
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Entry {
    pub fn new(title: &str, now_ms: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: now_ms,
            updated_at: now_ms,
            deleted: false,
            title: title.to_owned(),
            username: String::new(),
            password: String::new(),
            urls: Vec::new(),
            notes: String::new(),
            tags: Vec::new(),
            favorite: false,
            fields: Vec::new(),
            history: Vec::new(),
            extra: Extra::new(),
        }
    }

    /// Appends a history item and drops the oldest ones beyond [`HISTORY_LIMIT`].
    pub fn push_history(&mut self, item: HistoryItem) {
        self.history.push(item);
        let excess = self.history.len().saturating_sub(HISTORY_LIMIT);
        // Drained items are dropped, and `HistoryItem` zeroizes on drop.
        self.history.drain(..excess);
    }

    /// Brings `history` into canonical form (see [`normalized_history`]).
    /// Call after editing credentials; the merge relies on this invariant.
    pub fn normalize_history(&mut self) {
        let history = normalized_history(&self.history, self);
        self.history = history;
    }

    /// Turns the entry into a tombstone: keeps id and timestamps, wipes all content,
    /// including history, custom fields and unknown fields.
    pub fn delete(&mut self, now_ms: i64) {
        let id = self.id;
        let created_at = self.created_at;
        self.zeroize();
        self.extra.clear();
        self.id = id;
        self.created_at = created_at;
        self.updated_at = now_ms;
        self.deleted = true;
    }
}

impl HistoryItem {
    /// History item recording `entry`'s current credentials (an empty username is `None`).
    pub fn credentials_of(entry: &Entry, note: Option<&str>) -> Self {
        Self {
            at: entry.updated_at,
            password: entry.password.clone(),
            username: (!entry.username.is_empty()).then(|| entry.username.clone()),
            note: note.map(str::to_owned),
        }
    }

    /// Deduplication key. Two items with the same key describe the same credentials.
    fn key(&self) -> (&str, Option<&str>) {
        (&self.password, self.username.as_deref())
    }

    /// True when this item has the same key as the entry's current credentials.
    /// Exact key match (not "any username") keeps the merge associative.
    fn matches(&self, entry: &Entry) -> bool {
        let username = (!entry.username.is_empty()).then_some(entry.username.as_str());
        self.key() == (entry.password.as_str(), username)
    }

    fn is_empty(&self) -> bool {
        self.password.is_empty() && self.username.as_deref().is_none_or(str::is_empty)
    }
}

/// Canonical history for `current`:
/// 1. drop empty items and items equal to the current credentials;
/// 2. deduplicate by `(password, username)` (exact match, `None` is its own key),
///    keeping the greatest item;
/// 3. sort ascending by `(at, password, username, note)`;
/// 4. keep the newest [`HISTORY_LIMIT`].
///
/// The result depends only on the *set* of input items, which keeps the merge
/// commutative and idempotent.
pub fn normalized_history<'a>(items: impl IntoIterator<Item = &'a HistoryItem>, current: &Entry) -> Vec<HistoryItem> {
    let mut best: BTreeMap<(&str, Option<&str>), &HistoryItem> = BTreeMap::new();
    for item in items {
        if item.is_empty() || item.matches(current) {
            continue;
        }
        best.entry(item.key())
            .and_modify(|cur| {
                if item > *cur {
                    *cur = item;
                }
            })
            .or_insert(item);
    }
    let mut kept: Vec<&HistoryItem> = best.into_values().collect();
    kept.sort();
    let start = kept.len().saturating_sub(HISTORY_LIMIT);
    kept[start..].iter().map(|item| (*item).clone()).collect()
}
