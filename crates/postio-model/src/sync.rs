//! Per-mailbox synchronization state, and the decision it exists to answer.
//!
//! # Why this is one type
//!
//! `UIDVALIDITY`, `HIGHESTMODSEQ` and `UIDNEXT` are only meaningful *together*.
//! A `UID` cached under one `UIDVALIDITY` names a different message once the
//! server changes it, and a `MODSEQ` carried across that change would tell the
//! resync to skip exactly the messages it must re-fetch. Keeping the three in
//! separate fields that callers update one at a time is how mail clients grow
//! corruption bugs, so [`SyncState`] is written and read as a unit and the
//! decision that reads it — [`SyncState::plan`] — is a pure function of the
//! whole thing.
//!
//! # "Never synced" is a state, not a missing value
//!
//! A mailbox that has never been synchronized is not a mailbox whose
//! `uid_validity` happens to be `None`; it is a mailbox in a state the sync
//! engine has a specific plan for. [`SyncState::never_synced`] builds it and
//! [`SyncState::has_synced`] recognizes it, so no caller has to remember which
//! combination of `None`s means what.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, MailboxId, ModSeq, Uid, UidValidity};

/// What Postio knows about one mailbox's place in the server's UID space.
///
/// Persisted as a unit alongside the message writes it describes, so a crash
/// can never leave the state ahead of the messages it claims to cover. See the
/// [module docs](self) for why the three server counters travel together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The mailbox this describes.
    pub mailbox_id: MailboxId,
    /// Its account, denormalized so an account's state can be read in one query.
    pub account_id: AccountId,
    /// Generation of the mailbox's UID space. `None` means never synchronized.
    pub uid_validity: Option<UidValidity>,
    /// The UID the server said it would assign next, as of the last sync.
    pub uid_next: Option<Uid>,
    /// Highest `MODSEQ` seen, for QRESYNC/CONDSTORE incremental resync.
    pub highest_mod_seq: Option<ModSeq>,
    /// When a full synchronization of this mailbox last *completed*.
    pub last_full_sync_at: Option<DateTime<Utc>>,
    /// When the mailbox was last looked at on the server, complete or not.
    pub last_seen_at: Option<DateTime<Utc>>,
}

impl SyncState {
    /// The state of a mailbox that has never been synchronized.
    pub fn never_synced(mailbox_id: MailboxId, account_id: AccountId) -> Self {
        Self {
            mailbox_id,
            account_id,
            uid_validity: None,
            uid_next: None,
            highest_mod_seq: None,
            last_full_sync_at: None,
            last_seen_at: None,
        }
    }

    /// Whether a full synchronization has ever completed for this mailbox.
    ///
    /// Both halves are required: a mailbox that was selected once and then lost
    /// the connection has a `uid_validity` but no messages, and treating that
    /// as synchronized would leave the mailbox permanently half-empty.
    pub fn has_synced(&self) -> bool {
        self.uid_validity.is_some() && self.last_full_sync_at.is_some()
    }

    /// Whether every cached UID is stale under `observed`.
    pub fn uid_validity_changed(&self, observed: UidValidity) -> bool {
        matches!(self.uid_validity, Some(known) if known != observed)
    }

    /// What to do about this mailbox, given what the server just reported.
    ///
    /// The whole point of the type: one decision, made from the whole state at
    /// once, so no caller reimplements the precedence between a `UIDVALIDITY`
    /// change and a `MODSEQ` comparison.
    pub fn plan(&self, status: &MailboxStatus) -> ResyncPlan {
        // Ordered by severity. A UIDVALIDITY change outranks everything: under
        // it the stored MODSEQ describes a UID space that no longer exists.
        if self.uid_validity_changed(status.uid_validity) {
            return ResyncPlan::Full(FullResyncReason::UidValidityChanged);
        }
        if !self.has_synced() {
            return ResyncPlan::Full(FullResyncReason::NeverSynced);
        }

        let (Some(known), Some(reported)) = (self.highest_mod_seq, status.highest_mod_seq) else {
            // No CONDSTORE on one side or the other: there is no way to ask for
            // "what changed", so the only correct answer is to look at
            // everything.
            return ResyncPlan::Full(FullResyncReason::NoModSeq);
        };

        if reported > known {
            ResyncPlan::Incremental { since: known }
        } else if reported < known {
            // MODSEQ is defined to be monotonic, so a smaller one means the
            // server's store was rebuilt underneath us. Trusting ours would
            // skip every change since. Start over.
            ResyncPlan::Full(FullResyncReason::ModSeqWentBackwards)
        } else {
            ResyncPlan::UpToDate
        }
    }

    /// Records what the server reported when the mailbox was selected.
    ///
    /// Advances `last_seen_at` but never `last_full_sync_at`: selecting a
    /// mailbox is not synchronizing it. When `UIDVALIDITY` has changed, every
    /// cached counter is dropped along with it — keeping a `MODSEQ` from the
    /// previous UID space is the corruption this type exists to prevent.
    pub fn observe(&mut self, status: &MailboxStatus, at: DateTime<Utc>) {
        if self.uid_validity_changed(status.uid_validity) {
            self.highest_mod_seq = None;
            self.uid_next = None;
            self.last_full_sync_at = None;
        }
        self.uid_validity = Some(status.uid_validity);
        if let Some(uid_next) = status.uid_next {
            self.uid_next = Some(uid_next);
        }
        if let Some(mod_seq) = status.highest_mod_seq {
            self.highest_mod_seq = Some(mod_seq);
        }
        self.last_seen_at = Some(at);
    }

    /// Records that a full synchronization completed at `at`.
    pub fn complete_full_sync(&mut self, at: DateTime<Utc>) {
        self.last_full_sync_at = Some(at);
        self.last_seen_at = Some(at);
    }
}

/// What the server reported for a mailbox, from `SELECT` or `STATUS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStatus {
    /// The mailbox's current UID generation. Always present: a server that
    /// selects a mailbox must report it.
    pub uid_validity: UidValidity,
    /// The next UID the server will assign, when it said.
    pub uid_next: Option<Uid>,
    /// The mailbox's highest `MODSEQ`, when the server supports CONDSTORE.
    pub highest_mod_seq: Option<ModSeq>,
}

impl MailboxStatus {
    /// A report carrying only `UIDVALIDITY`, as a server without CONDSTORE
    /// gives.
    pub fn new(uid_validity: UidValidity) -> Self {
        Self {
            uid_validity,
            uid_next: None,
            highest_mod_seq: None,
        }
    }

    /// Adds the reported `UIDNEXT`.
    pub fn with_uid_next(mut self, uid_next: Uid) -> Self {
        self.uid_next = Some(uid_next);
        self
    }

    /// Adds the reported `HIGHESTMODSEQ`.
    pub fn with_highest_mod_seq(mut self, mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = Some(mod_seq);
        self
    }
}

/// How a mailbox should be brought up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResyncPlan {
    /// Re-enumerate the mailbox from scratch and discard cached UIDs.
    Full(FullResyncReason),
    /// Ask only for what changed since `since`.
    Incremental {
        /// The `MODSEQ` to ask the server for changes after.
        since: ModSeq,
    },
    /// Nothing to do: the server reports what we already have.
    UpToDate,
}

impl ResyncPlan {
    /// Whether this plan means re-enumerating the mailbox.
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full(_))
    }
}

/// Why a full resynchronization is required.
///
/// Carried rather than collapsed to a bool because it is what the sync-status
/// UI shows the user and what a bug report needs: "your server renumbered the
/// mailbox" and "this mailbox is new" produce identical traffic and completely
/// different explanations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FullResyncReason {
    /// Nothing has ever been synchronized for this mailbox.
    NeverSynced,
    /// The server changed `UIDVALIDITY`; every cached UID is stale.
    UidValidityChanged,
    /// One side does not support CONDSTORE, so "what changed" cannot be asked.
    NoModSeq,
    /// The server reported a `MODSEQ` below the one already seen, which the
    /// protocol says cannot happen — its store was rebuilt.
    ModSeqWentBackwards,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
    }

    fn synced() -> SyncState {
        let mut state = SyncState::never_synced(MailboxId::new(1), AccountId::new(1));
        state.uid_validity = Some(UidValidity::new(1_707_000_000));
        state.uid_next = Some(Uid::new(4_412));
        state.highest_mod_seq = Some(ModSeq::new(90_210));
        state.complete_full_sync(at(9));
        state
    }

    #[test]
    fn a_never_synced_mailbox_says_so() {
        let state = SyncState::never_synced(MailboxId::new(1), AccountId::new(1));

        assert!(!state.has_synced());
        assert_eq!(
            state.plan(&MailboxStatus::new(UidValidity::new(7))),
            ResyncPlan::Full(FullResyncReason::NeverSynced)
        );
    }

    #[test]
    fn selecting_a_mailbox_once_is_not_having_synced_it() {
        let mut state = SyncState::never_synced(MailboxId::new(1), AccountId::new(1));
        state.observe(&MailboxStatus::new(UidValidity::new(7)), at(9));

        assert!(
            !state.has_synced(),
            "a UIDVALIDITY without a completed sync is a half-empty mailbox"
        );
        assert_eq!(state.last_seen_at, Some(at(9)));
        assert_eq!(state.last_full_sync_at, None);
    }

    #[test]
    fn a_higher_server_mod_seq_asks_only_for_what_changed() {
        let state = synced();
        let status = MailboxStatus::new(UidValidity::new(1_707_000_000))
            .with_highest_mod_seq(ModSeq::new(90_300));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Incremental {
                since: ModSeq::new(90_210)
            }
        );
    }

    #[test]
    fn an_unchanged_mod_seq_is_nothing_to_do() {
        let state = synced();
        let status = MailboxStatus::new(UidValidity::new(1_707_000_000))
            .with_highest_mod_seq(ModSeq::new(90_210));

        assert_eq!(state.plan(&status), ResyncPlan::UpToDate);
        assert!(!state.plan(&status).is_full());
    }

    #[test]
    fn a_uid_validity_change_outranks_a_matching_mod_seq() {
        let state = synced();
        // Same MODSEQ as we hold: on its own that would read as up to date.
        let status = MailboxStatus::new(UidValidity::new(1_800_000_000))
            .with_highest_mod_seq(ModSeq::new(90_210));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::UidValidityChanged)
        );
    }

    #[test]
    fn a_server_without_condstore_forces_a_full_pass() {
        let state = synced();
        let status = MailboxStatus::new(UidValidity::new(1_707_000_000));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::NoModSeq)
        );
    }

    #[test]
    fn a_mod_seq_that_went_backwards_forces_a_full_pass() {
        let state = synced();
        let status = MailboxStatus::new(UidValidity::new(1_707_000_000))
            .with_highest_mod_seq(ModSeq::new(90_000));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::ModSeqWentBackwards)
        );
    }

    #[test]
    fn observing_a_new_uid_validity_drops_the_counters_that_belonged_to_the_old_one() {
        let mut state = synced();
        state.observe(&MailboxStatus::new(UidValidity::new(1_800_000_000)), at(10));

        assert_eq!(state.uid_validity, Some(UidValidity::new(1_800_000_000)));
        assert_eq!(state.highest_mod_seq, None, "belonged to the old UID space");
        assert_eq!(state.uid_next, None, "belonged to the old UID space");
        assert!(!state.has_synced(), "the mailbox has to be re-enumerated");
        assert_eq!(state.last_seen_at, Some(at(10)));
    }

    #[test]
    fn observing_the_same_uid_validity_advances_the_counters() {
        let mut state = synced();
        let status = MailboxStatus::new(UidValidity::new(1_707_000_000))
            .with_uid_next(Uid::new(4_500))
            .with_highest_mod_seq(ModSeq::new(90_300));
        state.observe(&status, at(10));

        assert_eq!(state.uid_next, Some(Uid::new(4_500)));
        assert_eq!(state.highest_mod_seq, Some(ModSeq::new(90_300)));
        assert!(state.has_synced(), "a completed sync is still completed");
        assert_eq!(state.last_full_sync_at, Some(at(9)), "not a full sync");
    }

    #[test]
    fn a_status_that_omits_a_counter_does_not_erase_it() {
        let mut state = synced();
        state.observe(&MailboxStatus::new(UidValidity::new(1_707_000_000)), at(10));

        assert_eq!(
            state.highest_mod_seq,
            Some(ModSeq::new(90_210)),
            "the server did not say, which is not the same as saying none"
        );
        assert_eq!(state.uid_next, Some(Uid::new(4_412)));
    }
}
