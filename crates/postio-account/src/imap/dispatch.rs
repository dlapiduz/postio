//! Capability checks happen once here and come out as a value to match on; pure, so the fallbacks
//! are testable without a server.

use crate::backend::{Capabilities, Capability};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStrategy {
    Move,
    /// Three round trips and not atomic: a crash between copy and store leaves the message in both
    /// mailboxes.
    CopyThenDelete {
        /// Without UIDPLUS a plain `EXPUNGE` would remove messages another client marked
        /// `\Deleted`, so the removal is left to the server's policy.
        uid_expunge: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpungeStrategy {
    UidExpunge,
    Expunge,
    /// Chosen when a targeted expunge was asked for and the server cannot do one: destroying
    /// another client's messages is worse than leaving ours.
    Defer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResyncStrategy {
    QResync,
    /// Finds changes cheaply but not deletions: a full UID listing is still needed.
    CondStore,
    FullUidScan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchStrategy {
    Idle,
    Poll,
}

/// `LIST ... RETURN (SUBSCRIBED)` (RFC 5258) is not offered: `io-imap` does not expose return
/// options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingStrategy {
    ListAndLsub,
    ListOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    capabilities: Capabilities,
}

impl Dispatch {
    pub fn new(capabilities: Capabilities) -> Self {
        Self { capabilities }
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn move_strategy(&self) -> MoveStrategy {
        if self.supports(Capability::Move) {
            MoveStrategy::Move
        } else {
            MoveStrategy::CopyThenDelete {
                uid_expunge: self.supports(Capability::UidPlus),
            }
        }
    }

    /// `targeted` is whether the caller named UIDs. Without UIDPLUS that is `Defer`: a bare
    /// `EXPUNGE` would destroy another client's `\Deleted` messages.
    pub fn expunge_strategy(&self, targeted: bool) -> ExpungeStrategy {
        match (targeted, self.supports(Capability::UidPlus)) {
            (true, true) => ExpungeStrategy::UidExpunge,
            (true, false) => ExpungeStrategy::Defer,
            (false, _) => ExpungeStrategy::Expunge,
        }
    }

    pub fn resync_strategy(&self) -> ResyncStrategy {
        if self.capabilities.supports_incremental_sync() {
            ResyncStrategy::QResync
        } else if self.supports(Capability::CondStore) {
            ResyncStrategy::CondStore
        } else {
            ResyncStrategy::FullUidScan
        }
    }

    pub fn watch_strategy(&self) -> WatchStrategy {
        if self.supports(Capability::Idle) {
            WatchStrategy::Idle
        } else {
            WatchStrategy::Poll
        }
    }

    /// `LSUB` is only worth its round trip when subscription state is used.
    pub fn listing_strategy(&self, subscribed_only: bool) -> ListingStrategy {
        if subscribed_only {
            ListingStrategy::ListAndLsub
        } else {
            ListingStrategy::ListOnly
        }
    }

    /// Without UIDPLUS the destination UID must be found by searching.
    pub fn reports_destination_uid(&self) -> bool {
        self.supports(Capability::UidPlus)
    }

    /// Sending `ENABLE` to a server that does not advertise it is a protocol error (RFC 5161).
    pub fn can_enable_extensions(&self) -> bool {
        self.supports(Capability::Enable)
    }

    /// QRESYNC implies CONDSTORE (RFC 7162 §3.2.5), but both are named: a server may enable only
    /// what was asked.
    pub fn extensions_to_enable(&self) -> Vec<&'static str> {
        if !self.can_enable_extensions() {
            return Vec::new();
        }
        let mut wanted = Vec::new();
        if self.supports(Capability::CondStore) {
            wanted.push(Capability::CondStore.as_str());
        }
        if self.supports(Capability::QResync) {
            wanted.push(Capability::QResync.as_str());
        }
        wanted
    }

    /// iCloud does not, so folder roles fall back to matching names.
    pub fn advertises_special_use(&self) -> bool {
        self.supports(Capability::SpecialUse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(names: &[&str]) -> Dispatch {
        Dispatch::new(Capabilities::from_names(names.iter().copied()))
    }

    fn full_featured() -> Dispatch {
        dispatch(&[
            "IMAP4rev1",
            "ENABLE",
            "CONDSTORE",
            "QRESYNC",
            "IDLE",
            "UIDPLUS",
            "MOVE",
            "NAMESPACE",
            "UNSELECT",
        ])
    }

    /// "We never send a bare EXPUNGE after a move" must fail a run if it stops being true (RFC 4315
    /// §2.1, #681).
    #[test]
    fn a_move_never_falls_back_to_an_untargeted_expunge() {
        assert_eq!(
            dispatch(&["IMAP4rev1", "UIDPLUS"]).move_strategy(),
            MoveStrategy::CopyThenDelete { uid_expunge: true },
            "with UIDPLUS the removal names the UIDs it means"
        );
        assert_eq!(
            dispatch(&["IMAP4rev1"]).move_strategy(),
            MoveStrategy::CopyThenDelete { uid_expunge: false },
            "without it Postio declines to expunge rather than expunging the \
             whole mailbox — the source copy is left flagged `\\Deleted`, \
             which is the named exception in docs/rfc-compliance.md"
        );
    }

    #[test]
    fn a_full_featured_server_gets_every_fast_path() {
        let dispatch = full_featured();

        assert_eq!(dispatch.move_strategy(), MoveStrategy::Move);
        assert_eq!(dispatch.expunge_strategy(true), ExpungeStrategy::UidExpunge);
        assert_eq!(dispatch.resync_strategy(), ResyncStrategy::QResync);
        assert_eq!(dispatch.watch_strategy(), WatchStrategy::Idle);
        assert!(dispatch.reports_destination_uid());
        assert_eq!(dispatch.extensions_to_enable(), ["CONDSTORE", "QRESYNC"]);
    }

    #[test]
    fn a_bare_imap4rev1_server_still_works_on_the_floor_path() {
        let dispatch = dispatch(&["IMAP4rev1"]);

        assert_eq!(
            dispatch.move_strategy(),
            MoveStrategy::CopyThenDelete { uid_expunge: false }
        );
        assert_eq!(dispatch.resync_strategy(), ResyncStrategy::FullUidScan);
        assert_eq!(dispatch.watch_strategy(), WatchStrategy::Poll);
        assert!(!dispatch.reports_destination_uid());
        assert!(dispatch.extensions_to_enable().is_empty());
    }

    #[test]
    fn lsub_is_only_paid_for_when_subscription_state_is_wanted() {
        let dispatch = full_featured();

        assert_eq!(
            dispatch.listing_strategy(true),
            ListingStrategy::ListAndLsub
        );
        assert_eq!(dispatch.listing_strategy(false), ListingStrategy::ListOnly);
    }

    #[test]
    fn uidplus_alone_makes_the_copy_fallback_safe_to_expunge() {
        let dispatch = dispatch(&["IMAP4rev1", "UIDPLUS"]);

        assert_eq!(
            dispatch.move_strategy(),
            MoveStrategy::CopyThenDelete { uid_expunge: true }
        );
    }

    #[test]
    fn a_targeted_expunge_is_deferred_rather_than_widened() {
        // Without UIDPLUS the only expunge removes every `\Deleted` message, another client's
        // included; leaving ours in place is the lesser harm.
        let dispatch = dispatch(&["IMAP4rev1"]);

        assert_eq!(dispatch.expunge_strategy(true), ExpungeStrategy::Defer);
        assert_eq!(dispatch.expunge_strategy(false), ExpungeStrategy::Expunge);
    }

    #[test]
    fn condstore_without_qresync_finds_changes_but_not_deletions() {
        let dispatch = dispatch(&["IMAP4rev1", "ENABLE", "CONDSTORE"]);

        assert_eq!(dispatch.resync_strategy(), ResyncStrategy::CondStore);
        assert_eq!(dispatch.extensions_to_enable(), ["CONDSTORE"]);
    }

    #[test]
    fn qresync_without_condstore_is_not_treated_as_incremental() {
        // RFC 7162 defines them together; half the pair cannot supply MODSEQ.
        let dispatch = dispatch(&["IMAP4rev1", "ENABLE", "QRESYNC"]);

        assert_eq!(dispatch.resync_strategy(), ResyncStrategy::FullUidScan);
    }

    #[test]
    fn enable_is_not_sent_to_a_server_that_does_not_advertise_it() {
        let dispatch = dispatch(&["IMAP4rev1", "CONDSTORE", "QRESYNC"]);

        assert!(!dispatch.can_enable_extensions());
        assert!(dispatch.extensions_to_enable().is_empty());
    }

    #[test]
    fn a_server_without_special_use_makes_roles_fall_back_to_names() {
        assert!(!full_featured().advertises_special_use());
        assert!(dispatch(&["IMAP4rev1", "SPECIAL-USE"]).advertises_special_use());
    }
}
