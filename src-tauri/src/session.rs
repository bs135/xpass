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
        report_key_lock(&vault);
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

    /// Takes a copy reconciled by sync (already written to disk by the caller).
    pub fn adopt(&mut self, vault: UnlockedVault) {
        self.vault = vault;
        self.refresh_clock();
    }

    /// D2: after a merge the clock must stay ahead of every timestamp seen.
    pub fn refresh_clock(&mut self) {
        self.last_seen_ts = self.last_seen_ts.max(self.vault.payload.max_updated_at());
    }

    /// Applies an edit to the payload, saves and writes the file atomically.
    /// `edit` receives the D2 timestamp to stamp on changed entries.
    /// On any error (from `edit` or I/O) the session keeps its previous state.
    pub fn apply<T>(
        &mut self,
        storage: &Storage,
        now_ms: i64,
        edit: impl FnOnce(&mut Payload, i64) -> AppResult<T>,
    ) -> AppResult<T> {
        let ts = next_timestamp(now_ms, self.last_seen_ts);
        let out = self.persist_with(storage, now_ms, |vault| edit(&mut vault.payload, ts))?;
        self.last_seen_ts = ts.max(self.vault.payload.max_updated_at());
        Ok(out)
    }

    /// Changes a copy of the vault (payload or key slots), saves and writes it,
    /// then swaps it in. On any error the session keeps its previous state.
    pub fn persist_with<T>(
        &mut self,
        storage: &Storage,
        now_ms: i64,
        change: impl FnOnce(&mut UnlockedVault) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut next = self.vault.clone();
        let out = change(&mut next)?;
        let bytes = next.save(&mut OsRng)?;
        storage.write(&bytes, now_ms)?;
        self.vault = next;
        Ok(out)
    }
}

/// Logs once per process when the VaultKey page could not be locked in RAM
/// (v0.6 H6): the vault still works, but the key may reach the pagefile.
fn report_key_lock(vault: &UnlockedVault) {
    static REPORTED: std::sync::Once = std::sync::Once::new();
    if !vault.key_locked() {
        REPORTED.call_once(|| eprintln!("xpass: VirtualLock failed; the vault key is not locked in RAM"));
    }
}
