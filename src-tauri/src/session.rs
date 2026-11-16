//! An unlocked vault plus the D2 clock state. Dropping it wipes the VaultKey and
//! the payload secrets (`UnlockedVault` zeroizes on drop).

use rand::rngs::OsRng;
use xpass_core::{clock::next_timestamp, model::Payload, UnlockedVault};

use crate::{error::AppResult, storage::Storage};

pub struct VaultSession {
    vault: UnlockedVault,
    /// Highest `updated_at` seen in this vault (D2).
    pub(crate) last_seen_ts: i64,
    /// Unlocked with the recovery key: the user may set a new password and a new
    /// Emergency Kit without the (forgotten) current password until the next lock.
    recovered: bool,
}

impl VaultSession {
    pub fn new(vault: UnlockedVault, recovered: bool) -> Self {
        let last_seen_ts = vault.payload.max_updated_at();
        Self {
            vault,
            last_seen_ts,
            recovered,
        }
    }

    pub fn recovered(&self) -> bool {
        self.recovered
    }

    pub fn vault(&self) -> &UnlockedVault {
        &self.vault
    }

    /// Applies an edit to the payload, saves and writes the file atomically.
    /// `edit` receives the D2 timestamp to stamp on changed entries.
    /// On any error the session keeps its previous state.
    #[allow(dead_code)] // Used by the entry commands (T6).
    pub fn apply(&mut self, storage: &Storage, now_ms: i64, edit: impl FnOnce(&mut Payload, i64)) -> AppResult<()> {
        let ts = next_timestamp(now_ms, self.last_seen_ts);
        self.persist_with(storage, now_ms, |vault| {
            edit(&mut vault.payload, ts);
            Ok(())
        })?;
        self.last_seen_ts = ts.max(self.vault.payload.max_updated_at());
        Ok(())
    }

    /// Changes a copy of the vault (payload or key slots), saves and writes it,
    /// then swaps it in. On any error the session keeps its previous state.
    pub fn persist_with(
        &mut self,
        storage: &Storage,
        now_ms: i64,
        change: impl FnOnce(&mut UnlockedVault) -> AppResult<()>,
    ) -> AppResult<()> {
        let mut next = self.vault.clone();
        change(&mut next)?;
        let bytes = next.save(&mut OsRng)?;
        storage.write(&bytes, now_ms)?;
        self.vault = next;
        Ok(())
    }
}
