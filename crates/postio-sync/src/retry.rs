//! Exponential backoff for failed queued operations. No jitter: the queue drains serially per
//! account, so there is no herd.
//! After `RetryPolicy::max_attempts` the operation is failed and the user told, never silently
//! dropped (docs/PRODUCT.md §16).

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub base: Duration,
    pub factor: u32,
    pub ceiling: Duration,
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    /// Two seconds, doubling, capped at five minutes, eight attempts: about twenty minutes, so a
    /// broken operation reaches the user in the same session.
    fn default() -> Self {
        Self {
            base: Duration::from_secs(2),
            factor: 2,
            ceiling: Duration::from_secs(300),
            max_attempts: 8,
        }
    }
}

impl RetryPolicy {
    /// `attempts` is how many were already made: the first call after a failure passes `1`. Zero is
    /// treated as one.
    pub fn backoff(&self, attempts: u32) -> Duration {
        let steps = attempts.saturating_sub(1);
        // Saturate: a large `factor` overflows long before the ceiling matters, and the ceiling is
        // the answer then.
        let multiplier = self
            .factor
            .checked_pow(steps)
            .map(u64::from)
            .unwrap_or(u64::MAX);
        self.base
            .checked_mul(multiplier.try_into().unwrap_or(u32::MAX))
            .unwrap_or(self.ceiling)
            .min(self.ceiling)
    }

    pub fn is_exhausted(&self, attempts: u32) -> bool {
        attempts >= self.max_attempts
    }

    /// A `Retry-After` shorter than our own backoff is ignored.
    pub fn next_attempt_at(
        &self,
        now: DateTime<Utc>,
        attempts: u32,
        server_asked_for: Option<Duration>,
    ) -> DateTime<Utc> {
        let wait = server_asked_for
            .unwrap_or_default()
            .max(self.backoff(attempts));
        now + TimeDelta::from_std(wait).unwrap_or(TimeDelta::MAX)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
    }

    #[test]
    fn the_wait_doubles_with_each_failure() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.backoff(1), Duration::from_secs(2));
        assert_eq!(policy.backoff(2), Duration::from_secs(4));
        assert_eq!(policy.backoff(3), Duration::from_secs(8));
        assert_eq!(policy.backoff(4), Duration::from_secs(16));
    }

    #[test]
    fn an_untried_operation_waits_the_base_rather_than_underflowing() {
        assert_eq!(RetryPolicy::default().backoff(0), Duration::from_secs(2));
    }

    #[test]
    fn the_wait_stops_at_the_ceiling() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.backoff(20), policy.ceiling);
        assert_eq!(
            policy.backoff(u32::MAX),
            policy.ceiling,
            "an absurd attempt count must not overflow into a short wait"
        );
    }

    #[test]
    fn a_huge_factor_still_lands_on_the_ceiling() {
        let policy = RetryPolicy {
            factor: 1_000,
            ..RetryPolicy::default()
        };

        assert_eq!(policy.backoff(6), policy.ceiling);
    }

    #[test]
    fn attempts_run_out() {
        let policy = RetryPolicy::default();

        assert!(!policy.is_exhausted(7));
        assert!(policy.is_exhausted(8));
        assert!(policy.is_exhausted(9), "and stay run out");
    }

    #[test]
    fn a_server_asking_for_longer_wins() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.next_attempt_at(at(9), 1, Some(Duration::from_secs(600))),
            at(9) + TimeDelta::seconds(600)
        );
    }

    #[test]
    fn a_server_asking_for_less_does_not_shorten_our_backoff() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.next_attempt_at(at(9), 5, Some(Duration::from_secs(1))),
            at(9) + TimeDelta::seconds(32),
            "six failures in, one second is not a good idea whoever suggests it"
        );
    }

    #[test]
    fn with_no_server_advice_the_backoff_stands() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.next_attempt_at(at(9), 3, None),
            at(9) + TimeDelta::seconds(8)
        );
    }
}
