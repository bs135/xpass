//! Delay between failed unlock attempts (in memory, reset on restart or success).
//!
//! After `n` consecutive failures the next attempt is allowed after
//! 0, 1, 2, 4, 8, 16, 30, 30, ... seconds.

use std::time::{Duration, Instant};

pub const MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct Throttle {
    failures: u32,
    not_before: Option<Instant>,
}

impl Throttle {
    /// `Err(wait)` while the delay after the last failure is still running.
    pub fn check(&self, now: Instant) -> Result<(), Duration> {
        match self.not_before {
            Some(t) if t > now => Err(t - now),
            _ => Ok(()),
        }
    }

    pub fn record_failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let delay = delay_after(self.failures);
        self.not_before = (!delay.is_zero()).then(|| now + delay);
    }

    pub fn record_success(&mut self) {
        *self = Self::default();
    }
}

/// Delay after `failures` consecutive failures.
pub fn delay_after(failures: u32) -> Duration {
    if failures <= 1 {
        return Duration::ZERO;
    }
    let secs = 1u64.checked_shl(failures - 2).unwrap_or(u64::MAX);
    Duration::from_secs(secs).min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_doubles_up_to_the_cap() {
        let secs: Vec<u64> = (0..9).map(|n| delay_after(n).as_secs()).collect();
        assert_eq!(secs, vec![0, 0, 1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(delay_after(u32::MAX), MAX_DELAY);
    }

    #[test]
    fn blocks_until_the_delay_passes_and_resets_on_success() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        t.record_failure(t0);
        assert_eq!(t.check(t0), Ok(()), "first failure: no delay");
        t.record_failure(t0);
        assert_eq!(t.check(t0), Err(Duration::from_secs(1)));
        assert_eq!(t.check(t0 + Duration::from_secs(1)), Ok(()));
        t.record_success();
        t.record_failure(t0);
        assert_eq!(t.check(t0), Ok(()), "counter reset after success");
    }
}
