//! When to stop trusting a server's change feed and read every flag.
//!
//! # Why this exists
//!
//! `CHANGEDSINCE` is how an incremental resync learns that a flag moved. It
//! is only as good as the server's bookkeeping, and some servers are known to
//! be bad at it: iCloud, Yahoo and Exchange under-report CONDSTORE changes,
//! and iCloud never pushes a flag change to an `IDLE` session. A flag set on
//! another device is then missed, and since the stored `HIGHESTMODSEQ`
//! advances past it anyway, missed for good.
//!
//! The cure is to stop asking *what changed* and ask *what is*: a flags-only
//! listing of the whole folder (`UID FETCH 1:* (UID FLAGS)`), compared with
//! the store. That costs a few dozen bytes per message, so how often to pay it
//! is a trade, and [`SweepPolicy::decide`] is where the trade is made. It is a
//! pure function of the folder's size, the passes since the last sweep and
//! how far the server's feed is trusted, so it is cheap to test and the
//! thresholds are data in [`SweepPolicy`], not constants at the call site.
//!
//! The decision is deliberately separate from [`SyncState::plan`], which
//! answers a different question (what the *mailbox* needs) from state the
//! server just reported. This one is about the server itself.
//!
//! [`SyncState::plan`]: crate::SyncState::plan

use serde::{Deserialize, Serialize};

/// How far a server's `CHANGEDSINCE` answer can be believed.
///
/// A property of the provider, so it lives in the provider preset table
/// (`condstore_reliable`), and a backend reports it with
/// `MailBackend::change_feed`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeFeed {
    /// The feed is complete, as RFC 7162 requires. The default: a server is
    /// believed until it is known not to deserve it.
    #[default]
    Reliable,
    /// The feed is known to miss flag changes.
    Unreliable,
}

/// What a resync pass does about flags, besides asking for the change feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagPass {
    /// Believe the change feed.
    Trust,
    /// Read every flag in the folder and reconcile the store with it.
    Sweep,
}

/// The thresholds of [`SweepPolicy::decide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepPolicy {
    /// A folder holding at most this many messages is swept on every pass:
    /// at this size the listing is tens of kilobytes, cheaper than being
    /// wrong.
    pub every_pass_up_to: u32,
    /// A larger folder is swept once in this many passes. The passes in
    /// between still use the change feed, which catches most changes at no
    /// extra cost. A value of 0 or 1 sweeps every pass.
    pub larger_every: u32,
}

impl SweepPolicy {
    /// What ships: the figures Aerion settled on, which are what a mailbox of
    /// ordinary size makes cheap (2000 messages is one short response) and a
    /// large archive makes worth rationing (a sweep of 60,000 is not).
    pub const DEFAULT: Self = Self {
        every_pass_up_to: 2_000,
        larger_every: 10,
    };

    /// Whether this pass should sweep.
    ///
    /// `exists` is the folder's message count as the server just reported it.
    /// `passes_since_sweep` is how many passes have run since the last sweep
    /// without one: 0 right after a sweep. A server whose feed is
    /// [`Reliable`](ChangeFeed::Reliable) is never swept, because the sweep
    /// would be paid for on every folder of every account to guard against a
    /// fault that server does not have.
    ///
    /// An unreliable server's *large* folder sweeps on the pass that makes
    /// `larger_every` of them since the last sweep, so with 10, nine passes
    /// trust the feed and the tenth sweeps.
    pub fn decide(&self, exists: u32, passes_since_sweep: u32, feed: ChangeFeed) -> FlagPass {
        match feed {
            ChangeFeed::Reliable => FlagPass::Trust,
            ChangeFeed::Unreliable if exists <= self.every_pass_up_to => FlagPass::Sweep,
            ChangeFeed::Unreliable if passes_since_sweep.saturating_add(1) >= self.larger_every => {
                FlagPass::Sweep
            }
            ChangeFeed::Unreliable => FlagPass::Trust,
        }
    }
}

impl Default for SweepPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: SweepPolicy = SweepPolicy::DEFAULT;

    impl FlagPass {
        fn is_sweep(self) -> bool {
            self == FlagPass::Sweep
        }
    }

    #[test]
    fn a_reliable_server_is_never_swept() {
        for exists in [0, 1, 2_000, 2_001, 1_000_000] {
            for passes in [0, 9, 10, 1_000] {
                assert_eq!(
                    POLICY.decide(exists, passes, ChangeFeed::Reliable),
                    FlagPass::Trust,
                    "{exists} messages, {passes} passes"
                );
            }
        }
    }

    #[test]
    fn a_small_folder_on_an_unreliable_server_is_swept_every_pass() {
        for passes in [0, 1, 5] {
            assert_eq!(
                POLICY.decide(2_000, passes, ChangeFeed::Unreliable),
                FlagPass::Sweep
            );
        }
        assert_eq!(
            POLICY.decide(0, 0, ChangeFeed::Unreliable),
            FlagPass::Sweep,
            "an empty folder is the cheapest sweep there is"
        );
    }

    #[test]
    fn a_large_folder_on_an_unreliable_server_is_swept_every_nth_pass() {
        let passes: Vec<bool> = (0..10)
            .map(|since| {
                POLICY
                    .decide(2_001, since, ChangeFeed::Unreliable)
                    .is_sweep()
            })
            .collect();
        assert_eq!(
            passes,
            [
                false, false, false, false, false, false, false, false, false, true
            ],
            "nine passes trust the feed, the tenth sweeps"
        );
    }

    #[test]
    fn a_counter_that_overshot_still_sweeps() {
        assert!(
            POLICY
                .decide(50_000, u32::MAX, ChangeFeed::Unreliable)
                .is_sweep()
        );
    }

    #[test]
    fn the_thresholds_are_the_policys_and_not_the_functions() {
        let policy = SweepPolicy {
            every_pass_up_to: 10,
            larger_every: 3,
        };
        assert!(policy.decide(10, 0, ChangeFeed::Unreliable).is_sweep());
        assert!(!policy.decide(11, 0, ChangeFeed::Unreliable).is_sweep());
        assert!(!policy.decide(11, 1, ChangeFeed::Unreliable).is_sweep());
        assert!(policy.decide(11, 2, ChangeFeed::Unreliable).is_sweep());
    }

    #[test]
    fn a_policy_that_never_rations_sweeps_every_pass() {
        let policy = SweepPolicy {
            every_pass_up_to: 0,
            larger_every: 0,
        };
        assert!(policy.decide(5, 0, ChangeFeed::Unreliable).is_sweep());
        let policy = SweepPolicy {
            every_pass_up_to: 0,
            larger_every: 1,
        };
        assert!(policy.decide(5, 0, ChangeFeed::Unreliable).is_sweep());
    }
}
