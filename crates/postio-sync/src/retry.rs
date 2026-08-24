//! When to try a failed operation again, and when to stop.
//!
//! # Why exponential
//!
//! A server that is down is usually down for a while, and a client that keeps
//! asking every second is the client that gets rate-limited — or, on a laptop,
//! the one that keeps the radio awake and flattens the battery. Doubling the
//! wait costs a user nothing when the server comes back quickly and costs the
//! server nothing when it does not.
//!
//! # Why no jitter
//!
//! Jitter exists to break up a herd of clients retrying in lockstep. The
//! operation queue drains serially within one account, so there is no herd
//! here: one connection, one operation at a time. The place a herd can form is
//! reconnection across accounts, and that backoff belongs to the connection
//! loop rather than to individual queue rows.
//!
//! # Giving up is a decision, not a silence
//!
//! After [`RetryPolicy::max_attempts`] the operation is *failed*, which means
//! the user is told — see `Drainer` and docs/PRODUCT.md §16. It is never dropped:
//! silently discarding a mutation the user watched happen locally is how a
//! mail client loses mail.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

/// How long to wait between attempts at a queued operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The wait after the first failure.
    pub base: Duration,
    /// What each subsequent wait is multiplied by.
    pub factor: u32,
    /// The longest this will ever wait, however many attempts have failed.
    pub ceiling: Duration,
    /// How many attempts to make before giving up and reporting.
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    /// Two seconds, doubling, capped at five minutes, eight attempts.
    ///
    /// Eight attempts spans roughly twenty minutes — long enough to ride out a
    /// dropped Wi-Fi connection or a server restart, short enough that a
    /// genuinely broken operation reaches the user in the same session rather
    /// than the next one.
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
    /// How long to wait after `attempts` failures.
    ///
    /// `attempts` is how many have already been made, so the first call after
    /// a failure passes `1` and gets [`RetryPolicy::base`]. Zero is treated as
    /// one: an operation that has not been tried does not need a backoff, but
    /// asking for one must not underflow.
    pub fn backoff(&self, attempts: u32) -> Duration {
        let steps = attempts.saturating_sub(1);
        // Saturate rather than wrap: with a large `factor` the multiplier
        // overflows long before the ceiling matters, and the ceiling is the
        // answer in every one of those cases anyway.
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

    /// Whether this operation has had all the attempts it is going to get.
    pub fn is_exhausted(&self, attempts: u32) -> bool {
        attempts >= self.max_attempts
    }

    /// When to try again, honouring a server that asked for longer.
    ///
    /// A `Retry-After` shorter than our own backoff is ignored: the server
    /// asking us to come back in one second does not make one second a good
    /// idea after six failures.
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
