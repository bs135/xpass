//! `sync-state.json`: the engine's [`SyncState`] plus which remote it belongs to and
//! the last result. No secrets. A missing or damaged file means "never synced",
//! which is always safe: the next sync merges instead of overwriting (S3).

// Read and written by the sync worker (T20); only `reset` is used before that.
#![cfg_attr(not(test), allow(dead_code))]

use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};
use xpass_core::sync::SyncState;

use crate::{error::AppResult, settings};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredSyncState {
    /// `SyncSettings::remote_id` this state was recorded for.
    pub remote_id: String,
    #[serde(flatten)]
    pub state: SyncState,
    /// Unix ms of the last successful sync.
    pub last_sync_at: Option<i64>,
}

impl StoredSyncState {
    /// The engine state if it belongs to `remote_id`, otherwise a fresh one.
    pub fn for_remote(&self, remote_id: &str) -> SyncState {
        if self.remote_id == remote_id {
            self.state.clone()
        } else {
            SyncState::default()
        }
    }
}

pub fn load(path: &Path) -> StoredSyncState {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, state: &StoredSyncState) -> AppResult<()> {
    settings::write_json_atomic(path, state)
}

/// Forgets everything about the remote (new repository, branch or path).
pub fn reset(path: &Path) -> AppResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_fallbacks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sync-state.json");
        assert_eq!(load(&path), StoredSyncState::default());

        let s = StoredSyncState {
            remote_id: "o/r@:xpass/vault.xpv".into(),
            state: SyncState::synced("abc".into(), b"file"),
            last_sync_at: Some(5),
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["baseSha"], "abc", "engine state is flattened");

        assert_eq!(s.for_remote("o/r@:xpass/vault.xpv"), s.state);
        assert_eq!(s.for_remote("other"), SyncState::default());

        fs::write(&path, b"{broken").unwrap();
        assert_eq!(load(&path), StoredSyncState::default());
        reset(&path).unwrap();
        reset(&path).unwrap();
        assert!(!path.exists());
    }
}
