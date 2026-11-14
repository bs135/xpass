//! Decrypted payload. Serialized as JSON inside the ciphertext.
//!
//! Merge-ready from day one (IDEA.md §3.4): every entry has a stable `id`,
//! `updated_at`, and deletions are tombstones (`deleted = true`, secrets cleared)
//! so another device's copy cannot resurrect them. The merge itself is spike 3.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Payload {
    pub schema: u32,
    /// Incremented on every save; lets sync detect a stale overwrite.
    pub revision: u64,
    pub entries: Vec<Entry>,
}

impl Default for Payload {
    fn default() -> Self {
        Self { schema: SCHEMA_VERSION, revision: 0, entries: Vec::new() }
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
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
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
            url: String::new(),
            notes: String::new(),
            tags: Vec::new(),
        }
    }

    /// Turns the entry into a tombstone: keeps id and timestamps, wipes content.
    pub fn delete(&mut self, now_ms: i64) {
        let id = self.id;
        let created_at = self.created_at;
        self.zeroize();
        self.id = id;
        self.created_at = created_at;
        self.updated_at = now_ms;
        self.deleted = true;
    }
}
