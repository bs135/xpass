//! D2: HLC-lite timestamps for `updated_at`.
//!
//! A device whose clock is behind still produces timestamps newer than anything it
//! has already seen, so its edits are not silently lost to older-looking ones.
//! The app keeps `last_seen_ms` per vault session, seeded from
//! [`Payload::max_updated_at`](crate::model::Payload::max_updated_at) on unlock and
//! after every merge.

/// Timestamp (Unix ms) for the next edit: `max(now_ms, last_seen_ms + 1)`.
pub fn next_timestamp(now_ms: i64, last_seen_ms: i64) -> i64 {
    now_ms.max(last_seen_ms.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::next_timestamp;

    #[test]
    fn uses_wall_clock_when_ahead() {
        assert_eq!(next_timestamp(1_000, 500), 1_000);
    }

    #[test]
    fn stays_monotonic_when_clock_is_behind() {
        assert_eq!(next_timestamp(1_000, 5_000), 5_001);
        assert_eq!(next_timestamp(1_000, 1_000), 1_001);
    }

    #[test]
    fn does_not_overflow() {
        assert_eq!(next_timestamp(0, i64::MAX), i64::MAX);
    }
}
