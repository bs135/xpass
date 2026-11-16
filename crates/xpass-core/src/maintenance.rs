//! D3: periodic cleanup of the payload.

use crate::model::Payload;

/// Tombstones older than this are removed (D3). A device offline for longer may
/// resurrect entries deleted elsewhere; accepted trade-off for bounded growth.
pub const TOMBSTONE_TTL_MS: i64 = 180 * 24 * 60 * 60 * 1000;

/// Removes tombstones whose `updated_at` is older than `now_ms - ttl_ms`.
/// Returns the number of entries removed.
pub fn purge_tombstones(payload: &mut Payload, now_ms: i64, ttl_ms: i64) -> usize {
    let cutoff = now_ms.saturating_sub(ttl_ms);
    let before = payload.entries.len();
    payload.entries.retain(|e| !(e.deleted && e.updated_at < cutoff));
    before - payload.entries.len()
}
