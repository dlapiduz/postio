//! A server's `CHANGEDSINCE` feed is only as good as its bookkeeping: iCloud, Yahoo and Exchange
//! under-report changes and iCloud never pushes flag changes to `IDLE`, so a missed flag is missed
//! for good once `HIGHESTMODSEQ` advances. The cure is a flags-only listing of the whole folder
//! compared with the store; `SweepPolicy::decide` trades its cost, with thresholds as data.
//! Separate from `SyncState::plan`, which is about the mailbox.

use serde::{Deserialize, Serialize};

/// A property of the provider (preset `condstore_reliable`), reported by
/// `MailBackend::change_feed`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeFeed {
    /// RFC 7162 requires it complete; a server is believed until known otherwise.
    #[default]
    Reliable,
    Unreliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagPass {
    Trust,
    Sweep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepPolicy {
    /// At this size the listing is tens of kilobytes, cheaper than being wrong.
    pub every_pass_up_to: u32,
    /// Passes between sweeps still use the change feed. 0 or 1 sweeps every pass.
    pub larger_every: u32,
}

impl SweepPolicy {
    /// 2000 messages is one short response; a sweep of 60,000 is worth rationing.
    pub const DEFAULT: Self = Self {
        every_pass_up_to: 2_000,
        larger_every: 10,
    };

    /// `exists` is the folder's message count as just reported; `passes_since_sweep` is 0 right
    /// after a sweep. A `Reliable` server is never swept, since the sweep would be paid on every
    /// folder to guard against a fault it does not have. A large folder of an unreliable server
    /// sweeps once per `larger_every` passes.
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
