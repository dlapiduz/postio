//! Staying connected: detecting a drop, backing off, and knowing when to stop.
//!
//! # Why this is a state machine and not a retry loop
//!
//! Laptops sleep, Wi-Fi drops, phones move between towers. Recovery has to be
//! automatic — and it has to be automatic without hammering the server, which
//! is the failure mode that gets a client rate-limited or blocked. Three
//! distinct situations look identical to a naive `loop { connect().await }`:
//!
//! * **The server is briefly unreachable.** Back off and come back.
//! * **The network is gone entirely.** Do not burn attempts on it; there is
//!   nothing to reach and the operating system already knows when it returns.
//! * **The password is wrong.** Stop. Retrying a refused credential forever is
//!   how an account gets locked, and no amount of waiting will fix it.
//!
//! [`Supervisor`] keeps those apart, and [`Link`] is the answer the UI's status
//! line reads.
//!
//! # Flapping
//!
//! Resetting the attempt count the moment a connection succeeds sounds right
//! and is the bug: a link that comes up and drops again every two seconds
//! never gets past the first backoff step, so the client reconnects as fast as
//! the link can fail. So a success only clears the count once the connection
//! has *held* for [`ReconnectPolicy::stability`]. A flapping link therefore
//! keeps climbing the backoff until it settles, which is exactly the
//! convergence this bead asks for.
//!
//! # Jitter
//!
//! Unlike the operation queue — which drains serially inside one account and
//! has no herd to break up — reconnection does have one: every account, and on
//! a shared server every client, comes back at the same instant after an
//! outage. So the delay here is jittered. The entropy is an *argument* rather
//! than something this module reaches for, which is what makes the schedule
//! unit-testable rather than merely plausible.
//!
//! # NetworkManager
//!
//! [`Supervisor::set_network`] is the seam. Feeding it from NetworkManager's
//! D-Bus `state` signal belongs to the layer that already owns a D-Bus
//! connection and a main loop; this crate stays free of both, and works
//! correctly — just less promptly — when nobody ever calls it.
//!
//! What the signal is *for* is worth being exact about, because getting it
//! wrong in the obvious direction is worse than ignoring it. NetworkManager
//! reports on the link: an interface, a route, a DHCP lease. It says nothing
//! about whether a particular mail server is answering, and treating "the link
//! is up" as "the connection will work" would reset the backoff on every
//! Wi-Fi re-association while a server is down — turning a signal meant to
//! make recovery *prompt* into one that makes it *loud*. So a link-up signal
//! collapses the remaining wait and leaves the attempt count exactly where it
//! was: try now, and if that fails, carry on backing off from where we were.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_account::backend::{BackendError, MailBackend};

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// How long to wait between reconnection attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// The wait after the first failure.
    pub base: Duration,
    /// What each subsequent wait is multiplied by.
    pub factor: u32,
    /// The longest this will ever wait.
    pub ceiling: Duration,
    /// How long a connection has to hold before it counts as recovered.
    pub stability: Duration,
}

impl Default for ReconnectPolicy {
    /// One second, doubling, capped at two minutes, stable after thirty.
    ///
    /// Two minutes is short enough that a laptop waking from sleep has mail
    /// before the user has finished opening the lid, and long enough that a
    /// server down for an hour sees thirty attempts rather than three thousand.
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
    /// The un-jittered wait after `attempts` failures.
    ///
    /// `attempts` counts failures already made, so the first retry passes `1`
    /// and gets [`ReconnectPolicy::base`].
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

    /// The wait after `attempts` failures, jittered by `entropy`.
    ///
    /// Equal jitter: half the backoff, plus a random share of the other half.
    /// The floor matters — full jitter can return almost zero, which puts a
    /// client that has just been refused straight back on the server's door.
    ///
    /// `entropy` is any value the caller likes; only its low bits are used, and
    /// the same value always produces the same delay. Pass something random in
    /// production and a constant in a test.
    pub fn delay(&self, attempts: u32, entropy: u64) -> Duration {
        let backoff = self.backoff(attempts);
        let half = backoff / 2;
        let share = (entropy % (JITTER_STEPS + 1)) as u32;
        half + (half * share) / JITTER_STEPS as u32
    }

    /// Whether a connection that has held since `since` counts as recovered.
    pub fn has_stabilized(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        TimeDelta::from_std(self.stability)
            .map(|window| now - since >= window)
            .unwrap_or(true)
    }
}

/// The granularity of the jitter. Milliseconds would be finer than the
/// scheduler can honour anyway.
const JITTER_STEPS: u64 = 1_000;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Why the connection will not come back without the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// The server refused the credentials. The user has to supply new ones —
    /// for an app-specific password, that means minting another.
    Authentication(String),
    /// Something else that retrying cannot fix: a certificate that does not
    /// verify, a server that advertises no capabilities, a command refused
    /// outright.
    ///
    /// Deliberately not broken down further. [`BackendError`] is
    /// `#[non_exhaustive]` and asks callers to branch on its predicates rather
    /// than its variants, so that adding one cannot silently change how
    /// existing code retries. What matters here is only "retrying will not
    /// help", and the string carries the rest to the user.
    Unrecoverable(String),
}

impl Blocker {
    /// The message to put in front of the user.
    pub fn reason(&self) -> &str {
        match self {
            Self::Authentication(reason) | Self::Unrecoverable(reason) => reason,
        }
    }

    /// Whether the user needs to re-enter a password.
    pub fn needs_credentials(&self) -> bool {
        matches!(self, Self::Authentication(_))
    }
}

/// Where the connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Dropped, or never made, and waiting to try again.
    Waiting {
        /// How many attempts have failed.
        attempts: u32,
        /// When the next one is due.
        retry_at: DateTime<Utc>,
    },
    /// Connected.
    Online {
        /// When it came up. What [`ReconnectPolicy::stability`] is measured
        /// from.
        since: DateTime<Utc>,
    },
    /// The machine has no network. Not our backoff's problem.
    ///
    /// Distinct from [`Link::Waiting`] on purpose: there is nothing to retry
    /// against, so attempts are not spent and the status line can say
    /// "offline" rather than counting down to a reconnection that cannot
    /// succeed.
    Offline,
    /// Stopped, and waiting for the user.
    Blocked(Blocker),
}

impl Link {
    /// Whether commands can be issued right now.
    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online { .. })
    }

    /// Whether this will resolve itself given time.
    pub fn recovers_on_its_own(&self) -> bool {
        matches!(self, Self::Waiting { .. } | Self::Online { .. })
    }
}

/// What the operating system says about the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkState {
    /// Nobody has said. Treated as available: a client that refuses to try
    /// because it could not find NetworkManager is worse than one that tries
    /// and fails.
    #[default]
    Unknown,
    /// There is a route to the world.
    Up,
    /// There is not.
    Down,
}

impl NetworkState {
    fn permits_an_attempt(self) -> bool {
        !matches!(self, Self::Down)
    }
}

// ---------------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------------

/// Keeps a backend connected, and knows when to stop trying.
///
/// Drives nothing on its own: [`Supervisor::poll`] is called by whatever owns
/// the timer, which keeps this crate free of an async runtime and makes every
/// transition below testable at an exact instant.
#[derive(Debug)]
pub struct Supervisor {
    policy: ReconnectPolicy,
    link: Link,
    network: NetworkState,
    /// Failures since the last connection that held. Not reset by a success
    /// that did not last — see the module docs on flapping.
    attempts: u32,
}

impl Supervisor {
    /// A supervisor that has not connected yet.
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

    /// Where the connection stands.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The policy in force.
    pub fn policy(&self) -> ReconnectPolicy {
        self.policy
    }

    /// How many attempts have failed since the last connection that held.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Tells the supervisor what the operating system says about the network.
    ///
    /// Going down parks the link without spending an attempt.
    ///
    /// Coming *up* collapses whatever wait is in force, whether the link was
    /// parked at [`Link::Offline`] or merely backing off: the delay was
    /// measured against a network that is no longer the one we have, and
    /// waiting it out is exactly the lag that makes opening a laptop lid feel
    /// slow. It does not clear the attempt count, because **a link that is up
    /// is not a server that is reachable** — the operating system knows about
    /// the first and nothing about the second. So the next attempt happens at
    /// once, and if it fails the backoff carries on from where it was rather
    /// than starting again at one second, which is what stops a flapping
    /// interface turning into a reconnection storm.
    ///
    /// [`NetworkState::Unknown`] is not a link-up signal — NetworkManager
    /// stopped, or was never there — so it only un-parks a link that had been
    /// told the network was gone. Not knowing is not evidence.
    ///
    /// Has no effect while [`Link::Blocked`]: a wrong password is still wrong
    /// on a different network.
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
                // Already past due: there is no wait left to collapse, and
                // rewriting `retry_at` would report a transition that changed
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

    /// Makes the next [`poll`](Self::poll) attempt immediately, keeping the
    /// attempt count.
    fn retry_at(&mut self, now: DateTime<Utc>) -> Option<Link> {
        self.transition(Link::Waiting {
            attempts: self.attempts,
            retry_at: now,
        })
    }

    /// Records that a command failed, so a drop is noticed between polls.
    ///
    /// The drainer and the fetchers see the connection die before any
    /// reconnection timer does; this is how they say so. A transient error
    /// drops the link into its backoff, an authentication failure blocks it,
    /// and anything else is left alone — a refused `MOVE` is the operation's
    /// problem, not the connection's.
    pub fn observe(&mut self, error: &BackendError, now: DateTime<Utc>) -> Option<Link> {
        if error.is_authentication_failure() {
            return self.block(Blocker::Authentication(error.to_string()));
        }
        if !error.is_transient() || !self.link.is_online() {
            return None;
        }
        Some(self.fail(now, entropy_from(now)))
    }

    /// Clears a [`Link::Blocked`] so the supervisor will try again.
    ///
    /// What "the user typed a new password" means. The attempt count goes with
    /// it: this is a fresh start, not a continuation.
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

    /// Does whatever the clock says is due, and reports any change.
    ///
    /// Returns the new [`Link`] when it moved, and `None` when there was
    /// nothing to do — which is the common case and costs one comparison.
    ///
    /// `entropy` jitters the backoff; see [`ReconnectPolicy::delay`].
    pub async fn poll(
        &mut self,
        backend: &dyn MailBackend,
        now: DateTime<Utc>,
        entropy: u64,
    ) -> Option<Link> {
        match &self.link {
            // Both need the user, or the operating system. Neither needs us.
            Link::Blocked(_) | Link::Offline => return None,
            Link::Online { since } => {
                let since = *since;
                // A cheap liveness check: the backend answers from its own
                // session state, so this is not a round trip on a healthy
                // connection.
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
                // `attempts` is deliberately *not* cleared here. It clears once
                // the connection has held for the stability window, in `poll`
                // above — otherwise a flapping link never leaves the first
                // backoff step.
                self.transition(Link::Online { since: now })
            }
            Err(error) if error.is_authentication_failure() => {
                self.block(Blocker::Authentication(error.to_string()))
            }
            Err(error) if !error.is_transient() => {
                self.block(Blocker::Unrecoverable(error.to_string()))
            }
            Err(error) => {
                // A server that asked us to wait is obeyed, when it asked for
                // longer than we had planned.
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

    /// Counts a failure and schedules the next attempt.
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

/// Entropy for a jitter nobody supplied one for.
///
/// Only used by [`Supervisor::observe`], which is called from an error path
/// where threading a random value through would be noise. The sub-second part
/// of the clock is not cryptographic and does not need to be: it only has to
/// differ between two clients, and two clients do not share a nanosecond.
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

    // -- the schedule -----------------------------------------------------

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

        // Full jitter — `random(0, backoff)` — can put a client that was just
        // refused straight back on the server's door. Equal jitter cannot.
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

    // -- state ------------------------------------------------------------

    #[test]
    fn a_blocker_says_whether_the_user_has_to_do_something_about_a_password() {
        assert!(Blocker::Authentication("nope".into()).needs_credentials());
        assert!(!Blocker::Unrecoverable("nope".into()).needs_credentials());
        assert_eq!(Blocker::Unrecoverable("why".into()).reason(), "why");
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
