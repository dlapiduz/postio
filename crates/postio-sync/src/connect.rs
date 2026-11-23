//! Staying connected: a state machine, not a retry loop. A drop backs off with jitter, no network
//! waits, a refused credential stops.
//! A success clears the attempt count only once the link has held for `ReconnectPolicy::stability`,
//! so a flapping link keeps climbing the backoff.
//! `Supervisor::set_network` link-up collapses the wait but keeps the attempt count: the link says
//! nothing about the server.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_account::backend::{BackendError, MailBackend};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub base: Duration,
    pub factor: u32,
    pub ceiling: Duration,
    pub stability: Duration,
}

impl Default for ReconnectPolicy {
    /// One second, doubling, capped at two minutes, stable after thirty.
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            factor: 2,
            ceiling: Duration::from_secs(120),
            stability: Duration::from_secs(30),
        }
    }
}

impl ReconnectPolicy {
    /// `attempts` counts failures already made: the first retry passes `1` and gets `base`.
    pub fn backoff(&self, attempts: u32) -> Duration {
        let steps = attempts.saturating_sub(1);
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

    /// Equal jitter: half the backoff plus a share of the other half; full jitter could return
    /// almost zero and hammer a server that just refused.
    /// Only the low bits of `entropy` are used, and the same value gives the same delay.
    pub fn delay(&self, attempts: u32, entropy: u64) -> Duration {
        let backoff = self.backoff(attempts);
        let half = backoff / 2;
        let share = (entropy % (JITTER_STEPS + 1)) as u32;
        half + (half * share) / JITTER_STEPS as u32
    }

    pub fn has_stabilized(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        TimeDelta::from_std(self.stability)
            .map(|window| now - since >= window)
            .unwrap_or(true)
    }
}

const JITTER_STEPS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    Authentication(String),
    NoPassword(String),
    /// Retrying cannot fix it (bad certificate, no capabilities, a command refused outright).
    /// Not broken down: `BackendError` is `#[non_exhaustive]`, so callers branch on its predicates.
    Unrecoverable(String),
}

fn blocker_for_password(error: &BackendError) -> Blocker {
    if error.is_authentication_failure() {
        Blocker::Authentication(error.to_string())
    } else {
        Blocker::NoPassword(error.to_string())
    }
}

impl Blocker {
    pub fn reason(&self) -> &str {
        match self {
            Self::Authentication(reason)
            | Self::NoPassword(reason)
            | Self::Unrecoverable(reason) => reason,
        }
    }

    pub fn needs_credentials(&self) -> bool {
        matches!(self, Self::Authentication(_) | Self::NoPassword(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Waiting {
        attempts: u32,
        retry_at: DateTime<Utc>,
    },
    Online {
        /// When it came up. What [`ReconnectPolicy::stability`] is measured
        /// from.
        since: DateTime<Utc>,
    },
    /// No network: attempts are not spent, and the status line can say "offline".
    Offline,
    Blocked(Blocker),
}

impl Link {
    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online { .. })
    }

    pub fn recovers_on_its_own(&self) -> bool {
        matches!(self, Self::Waiting { .. } | Self::Online { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkState {
    /// Nobody has said; treated as available, since refusing to try for lack of NetworkManager is
    /// worse than failing.
    #[default]
    Unknown,
    Up,
    Down,
}

impl NetworkState {
    fn permits_an_attempt(self) -> bool {
        !matches!(self, Self::Down)
    }
}

/// Drives nothing itself: `poll` is called by whatever owns the timer, so this crate has no async
/// runtime.
#[derive(Debug)]
pub struct Supervisor {
    policy: ReconnectPolicy,
    link: Link,
    network: NetworkState,
    /// Failures since the last connection that held; a success that did not last does not reset it.
    attempts: u32,
}

impl Supervisor {
    pub fn new(policy: ReconnectPolicy) -> Self {
        Self {
            policy,
            link: Link::Waiting {
                attempts: 0,
                retry_at: DateTime::<Utc>::MIN_UTC,
            },
            network: NetworkState::Unknown,
            attempts: 0,
        }
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    pub fn policy(&self) -> ReconnectPolicy {
        self.policy
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Down parks the link without spending an attempt. Up collapses any wait in force but keeps
    /// the attempt count:
    /// a link that is up is not a server that is reachable, so a flapping interface must not become
    /// a reconnection storm.
    /// `Unknown` only un-parks a link told the network was gone. No effect while `Blocked`.
    pub fn set_network(&mut self, state: NetworkState, now: DateTime<Utc>) -> Option<Link> {
        if self.network == state {
            return None;
        }
        self.network = state;

        if matches!(self.link, Link::Blocked(_)) {
            return None;
        }

        match state {
            NetworkState::Down => self.transition(Link::Offline),
            NetworkState::Up => match self.link {
                Link::Offline => self.retry_at(now),
                // Already past due: rewriting `retry_at` would report a transition that changed
                // nothing.
                Link::Waiting { retry_at, .. } if retry_at > now => self.retry_at(now),
                _ => None,
            },
            NetworkState::Unknown => match self.link {
                Link::Offline => self.retry_at(now),
                _ => None,
            },
        }
    }

    fn retry_at(&mut self, now: DateTime<Utc>) -> Option<Link> {
        self.transition(Link::Waiting {
            attempts: self.attempts,
            retry_at: now,
        })
    }

    /// Records a failed command so a drop is noticed between polls. Transient drops into backoff,
    /// authentication blocks, anything else is the operation's problem, not the connection's.
    pub fn observe(&mut self, error: &BackendError, now: DateTime<Utc>) -> Option<Link> {
        if error.needs_a_password() {
            return self.block(blocker_for_password(error));
        }
        if !error.is_transient() || !self.link.is_online() {
            return None;
        }
        Some(self.fail(now, entropy_from(now)))
    }

    /// Clears a `Blocked` link; the attempt count goes too, as this is a fresh start.
    pub fn retry_now(&mut self, now: DateTime<Utc>) -> Option<Link> {
        if !matches!(self.link, Link::Blocked(_)) {
            return None;
        }
        self.attempts = 0;
        self.transition(Link::Waiting {
            attempts: 0,
            retry_at: now,
        })
    }

    /// Returns the new `Link` when it moved, `None` when there was nothing to do.
    pub async fn poll(
        &mut self,
        backend: &dyn MailBackend,
        now: DateTime<Utc>,
        entropy: u64,
    ) -> Option<Link> {
        match &self.link {
            Link::Blocked(_) | Link::Offline => return None,
            Link::Online { since } => {
                let since = *since;
                return match backend.capabilities().await {
                    Ok(_) => {
                        if self.policy.has_stabilized(since, now) {
                            self.attempts = 0;
                        }
                        None
                    }
                    Err(error) => self.observe(&error, now),
                };
            }
            Link::Waiting { retry_at, .. } => {
                if now < *retry_at {
                    return None;
                }
            }
        }

        if !self.network.permits_an_attempt() {
            return self.transition(Link::Offline);
        }

        match backend.connect().await {
            Ok(_) => {
                // `attempts` clears only once the connection has held for the stability window (in
                // `poll`), or a flapping link never leaves the first backoff step.
                self.transition(Link::Online { since: now })
            }
            Err(error) if error.needs_a_password() => self.block(blocker_for_password(&error)),
            Err(error) if !error.is_transient() => {
                self.block(Blocker::Unrecoverable(error.to_string()))
            }
            Err(error) => {
                // A server that asked for a longer wait than planned is obeyed.
                let wait = error.retry_after();
                let mut link = self.fail(now, entropy);
                if let (Some(asked), Link::Waiting { retry_at, attempts }) = (wait, &link) {
                    let floor = now + TimeDelta::from_std(asked).unwrap_or(TimeDelta::MAX);
                    if floor > *retry_at {
                        link = Link::Waiting {
                            attempts: *attempts,
                            retry_at: floor,
                        };
                        self.link = link.clone();
                    }
                }
                Some(link)
            }
        }
    }

    fn fail(&mut self, now: DateTime<Utc>, entropy: u64) -> Link {
        self.attempts = self.attempts.saturating_add(1);
        let delay = self.policy.delay(self.attempts, entropy);
        let link = Link::Waiting {
            attempts: self.attempts,
            retry_at: now + TimeDelta::from_std(delay).unwrap_or(TimeDelta::MAX),
        };
        self.link = link.clone();
        link
    }

    fn block(&mut self, blocker: Blocker) -> Option<Link> {
        self.transition(Link::Blocked(blocker))
    }

    fn transition(&mut self, link: Link) -> Option<Link> {
        if self.link == link {
            return None;
        }
        self.link = link.clone();
        Some(link)
    }
}

/// For `observe`, on an error path where threading a random value is noise; it only has to differ
/// between two clients.
fn entropy_from(now: DateTime<Utc>) -> u64 {
    now.timestamp_subsec_nanos() as u64
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second as i64)
    }

    #[test]
    fn the_backoff_doubles_and_stops_at_the_ceiling() {
        let policy = ReconnectPolicy::default();

        assert_eq!(policy.backoff(1), Duration::from_secs(1));
        assert_eq!(policy.backoff(2), Duration::from_secs(2));
        assert_eq!(policy.backoff(3), Duration::from_secs(4));
        assert_eq!(
            policy.backoff(8),
            Duration::from_secs(128).min(policy.ceiling)
        );
        assert_eq!(policy.backoff(30), policy.ceiling);
        assert_eq!(
            policy.backoff(u32::MAX),
            policy.ceiling,
            "an absurd attempt count must not overflow into a short wait"
        );
    }

    #[test]
    fn jitter_stays_between_half_the_backoff_and_all_of_it() {
        let policy = ReconnectPolicy::default();

        for attempts in 1..12 {
            let backoff = policy.backoff(attempts);
            for entropy in [0, 1, 7, 499, 500, 999, 1_000, u64::MAX] {
                let delay = policy.delay(attempts, entropy);
                assert!(
                    delay >= backoff / 2 && delay <= backoff,
                    "attempt {attempts}, entropy {entropy}: {delay:?} outside \
                     [{:?}, {backoff:?}]",
                    backoff / 2
                );
            }
        }
    }

    #[test]
    fn jitter_never_returns_almost_nothing() {
        let policy = ReconnectPolicy::default();

        assert_eq!(policy.delay(1, 0), policy.backoff(1) / 2);
    }

    #[test]
    fn the_same_entropy_always_gives_the_same_delay() {
        let policy = ReconnectPolicy::default();

        assert_eq!(policy.delay(4, 12_345), policy.delay(4, 12_345));
    }

    #[test]
    fn different_entropy_spreads_clients_out() {
        let policy = ReconnectPolicy::default();
        let spread: std::collections::BTreeSet<Duration> =
            (0..64).map(|seed| policy.delay(6, seed * 17)).collect();

        assert!(
            spread.len() > 32,
            "64 clients landed on only {} distinct delays",
            spread.len()
        );
    }

    #[test]
    fn a_connection_is_stable_only_after_the_window() {
        let policy = ReconnectPolicy::default();

        assert!(!policy.has_stabilized(at(0), at(29)));
        assert!(policy.has_stabilized(at(0), at(30)));
    }

    #[test]
    fn a_blocker_says_whether_the_user_has_to_do_something_about_a_password() {
        assert!(Blocker::Authentication("nope".into()).needs_credentials());
        assert!(!Blocker::Unrecoverable("nope".into()).needs_credentials());
        assert_eq!(Blocker::Unrecoverable("why".into()).reason(), "why");
    }

    #[test]
    fn a_missing_credential_blocks_the_link_for_a_password() {
        let mut state = Supervisor::new(ReconnectPolicy::default());
        let missing = postio_account::backend::BackendError::Secret(
            postio_account::secret::SecretError::NotFound {
                account: "ada@example.com".to_owned(),
            },
        );

        let moved = state.observe(&missing, at(0));

        let Some(Link::Blocked(blocker)) = moved else {
            panic!("a missing password must block the link, not retry it: {moved:?}")
        };
        assert!(matches!(blocker, Blocker::NoPassword(_)), "{blocker:?}");
        assert!(blocker.needs_credentials(), "so the user is asked for one");
    }

    #[test]
    fn a_dead_refresh_grant_blocks_the_link_for_the_user_to_fix() {
        // An expired grant (#954) must surface as blocked, needing credentials, not recovering on a
        // timer (ADR 0005 Q10): the same `Blocker::Authentication` a refused refresh gives.
        let mut state = Supervisor::new(ReconnectPolicy::default());
        let dead = postio_account::backend::BackendError::Secret(
            postio_account::secret::SecretError::GrantExpired {
                account: "ada@example.com".to_owned(),
            },
        );

        let moved = state.observe(&dead, at(0));

        assert!(
            matches!(moved, Some(Link::Blocked(Blocker::Authentication(_)))),
            "a dead grant must block the link, not retry it: {moved:?}"
        );
        let Some(Link::Blocked(blocker)) = moved else {
            unreachable!("asserted above")
        };
        assert!(blocker.needs_credentials(), "and say the user has to act");
        assert!(
            !Link::Blocked(blocker).recovers_on_its_own(),
            "no amount of waiting renews a grant the provider has retired"
        );
    }

    #[test]
    fn only_waiting_and_online_recover_on_their_own() {
        assert!(Link::Online { since: at(0) }.recovers_on_its_own());
        assert!(
            Link::Waiting {
                attempts: 1,
                retry_at: at(1)
            }
            .recovers_on_its_own()
        );
        assert!(!Link::Offline.recovers_on_its_own());
        assert!(!Link::Blocked(Blocker::Authentication("no".into())).recovers_on_its_own());
    }

    #[test]
    fn an_unknown_network_is_worth_trying() {
        assert!(NetworkState::default().permits_an_attempt());
        assert!(NetworkState::Up.permits_an_attempt());
        assert!(!NetworkState::Down.permits_an_attempt());
    }
}
