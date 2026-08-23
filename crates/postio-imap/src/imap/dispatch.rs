//! Choosing commands by what the server actually advertised.
//!
//! Every extension Postio's design leans on is optional, and iCloud is not the
//! only server it will ever meet. Rather than scattering `if
//! capabilities.contains(…)` through the command implementations, the choice
//! is made once, here, and comes out as a value the caller matches on.
//!
//! That has two payoffs. The fallbacks are testable without a server — this
//! module is pure — and the *cost* of a missing extension is written down
//! where someone reading it will see it, instead of being discovered when a
//! Fastmail user reports that archiving is slow.

use crate::backend::{Capabilities, Capability};

/// How to move messages between mailboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStrategy {
    /// `UID MOVE` (RFC 6851): one round trip, atomic on the server.
    Move,
    /// `UID COPY`, then `STORE \Deleted`, then an expunge.
    ///
    /// Three round trips, and not atomic: a crash between the copy and the
    /// store leaves the message in both mailboxes. The sync engine has to
    /// tolerate that, which is why this is a value and not a hidden branch.
    CopyThenDelete {
        /// Whether the expunge can be limited to the messages we deleted.
        ///
        /// Without UIDPLUS a plain `EXPUNGE` also removes messages *another
        /// client* marked `\Deleted` in the same mailbox. Postio does not do
        /// that: with this `false`, the copy is made and the `\Deleted` flag
        /// set, and the actual removal is left to the server's own policy.
        uid_expunge: bool,
    },
}

/// How to remove messages that are marked `\Deleted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpungeStrategy {
    /// `UID EXPUNGE` (RFC 4315): only the UIDs we name.
    UidExpunge,
    /// `EXPUNGE`: everything in the mailbox marked `\Deleted`, including by
    /// somebody else.
    Expunge,
    /// Do nothing, and let the server decide when to reclaim.
    ///
    /// Chosen when a targeted expunge was asked for and the server cannot do
    /// one: destroying another client's messages is worse than leaving ours.
    Defer,
}

/// How to bring a mailbox up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResyncStrategy {
    /// `SELECT (QRESYNC …)`: changes, vanishes and the new HIGHESTMODSEQ in
    /// one command. What epic E5 is designed around.
    QResync,
    /// `SELECT (CONDSTORE)` plus `UID FETCH … (CHANGEDSINCE n)`.
    ///
    /// Finds changes cheaply but not deletions: a full UID listing is still
    /// needed to spot what vanished.
    CondStore,
    /// Fetch every UID and compare against what is stored.
    ///
    /// The floor. Correct everywhere, proportional to mailbox size, and the
    /// reason CONDSTORE support is worth gating on.
    FullUidScan,
}

/// How to learn about new mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchStrategy {
    /// `IDLE` (RFC 2177): the server tells us.
    Idle,
    /// Poll with `STATUS`/`NOOP` on an interval.
    Poll,
}

/// How to find out which mailboxes exist and which are subscribed.
///
/// RFC 5258's `LIST … RETURN (SUBSCRIBED)` would collapse both into one round
/// trip, but `io-imap` does not expose the return options, so it is not a
/// choice this table can offer. Revisit when it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingStrategy {
    /// `LIST` for the folders and `LSUB` for the subscriptions, merged
    /// locally. Two round trips.
    ListAndLsub,
    /// `LIST` alone. Every folder is reported as subscribed, because the
    /// server was never asked otherwise.
    ListOnly,
}

/// The command choices one server's capability set implies.
///
/// Built once per session and consulted per command, so a capability check
/// never turns into a round trip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    capabilities: Capabilities,
}

impl Dispatch {
    /// Reads the choices out of a capability set.
    pub fn new(capabilities: Capabilities) -> Self {
        Self { capabilities }
    }

    /// The capability set these choices were derived from.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Whether the server advertised `capability`.
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(capability)
    }

    /// How to move messages.
    pub fn move_strategy(&self) -> MoveStrategy {
        if self.supports(Capability::Move) {
            MoveStrategy::Move
        } else {
            MoveStrategy::CopyThenDelete {
                uid_expunge: self.supports(Capability::UidPlus),
            }
        }
    }

    /// How to expunge.
    ///
    /// `targeted` is whether the caller named specific UIDs. A targeted
    /// expunge on a server without UIDPLUS is [`ExpungeStrategy::Defer`],
    /// because the alternative — a bare `EXPUNGE` — would also destroy
    /// messages another client marked `\Deleted` in the same mailbox.
    pub fn expunge_strategy(&self, targeted: bool) -> ExpungeStrategy {
        match (targeted, self.supports(Capability::UidPlus)) {
            (true, true) => ExpungeStrategy::UidExpunge,
            (true, false) => ExpungeStrategy::Defer,
            (false, _) => ExpungeStrategy::Expunge,
        }
    }

    /// How to resynchronize a mailbox.
    pub fn resync_strategy(&self) -> ResyncStrategy {
        if self.capabilities.supports_incremental_sync() {
            ResyncStrategy::QResync
        } else if self.supports(Capability::CondStore) {
            ResyncStrategy::CondStore
        } else {
            ResyncStrategy::FullUidScan
        }
    }

    /// How to watch for new mail.
    pub fn watch_strategy(&self) -> WatchStrategy {
        if self.supports(Capability::Idle) {
            WatchStrategy::Idle
        } else {
            WatchStrategy::Poll
        }
    }

    /// How to list mailboxes.
    ///
    /// The second round trip is only worth paying for when subscription state
    /// is going to be used: `LSUB` exists to answer "which folders did the
    /// user choose to see", and a listing that shows everything has no
    /// question to ask.
    pub fn listing_strategy(&self, subscribed_only: bool) -> ListingStrategy {
        if subscribed_only {
            ListingStrategy::ListAndLsub
        } else {
            ListingStrategy::ListOnly
        }
    }

    /// Whether `APPEND` and `COPY` report where the message landed.
    ///
    /// Without UIDPLUS the destination UID has to be found by searching, so
    /// callers get `None` rather than a guess.
    pub fn reports_destination_uid(&self) -> bool {
        self.supports(Capability::UidPlus)
    }

    /// Whether `ENABLE` may be sent at all.
    ///
    /// RFC 5161 requires the capability; sending `ENABLE` to a server that
    /// does not advertise it is a protocol error, not a harmless no-op.
    pub fn can_enable_extensions(&self) -> bool {
        self.supports(Capability::Enable)
    }

    /// The extensions to `ENABLE` on a fresh session, in wire spelling.
    ///
    /// QRESYNC implies CONDSTORE per RFC 7162 §3.2.5, but both are named:
    /// a server is entitled to enable only what it was asked for.
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

    /// Whether the server names its own special-use folders.
    ///
    /// iCloud does not, which is why folder roles fall back to matching
    /// names — see [`MailboxRole::resolve`](postio_model::MailboxRole::resolve).
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

    /// iCloud's documented post-authentication set.
    fn icloud() -> Dispatch {
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

    #[test]
    fn icloud_gets_every_fast_path() {
        let dispatch = icloud();

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
        let dispatch = icloud();

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
        // Without UIDPLUS the only expunge available removes every `\Deleted`
        // message in the mailbox, including ones another client marked.
        // Losing someone else's mail is worse than leaving ours in place.
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
        // RFC 7162 defines them together; a server advertising only half of
        // the pair cannot supply MODSEQ, so there is nothing to be
        // incremental about.
        let dispatch = dispatch(&["IMAP4rev1", "ENABLE", "QRESYNC"]);

        assert_eq!(dispatch.resync_strategy(), ResyncStrategy::FullUidScan);
    }

    #[test]
    fn enable_is_not_sent_to_a_server_that_does_not_advertise_it() {
        // RFC 5161 requires the capability; sending it regardless is a
        // protocol error rather than a harmless no-op.
        let dispatch = dispatch(&["IMAP4rev1", "CONDSTORE", "QRESYNC"]);

        assert!(!dispatch.can_enable_extensions());
        assert!(dispatch.extensions_to_enable().is_empty());
    }

    #[test]
    fn icloud_does_not_advertise_special_use_so_roles_fall_back_to_names() {
        assert!(!icloud().advertises_special_use());
        assert!(dispatch(&["IMAP4rev1", "SPECIAL-USE"]).advertises_special_use());
    }
}
