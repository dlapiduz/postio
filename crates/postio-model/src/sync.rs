//! `UIDVALIDITY`, `HIGHESTMODSEQ` and `UIDNEXT` are only meaningful together: a `MODSEQ` carried
//! across a `UIDVALIDITY` change would skip
//! exactly the messages that must be re-fetched. `SyncState` is written and read as a unit, and
//! `plan` is a pure function of it.
//! "Never synced" is a state (`never_synced`, `has_synced`), not a missing value.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, Generation, MailboxId, ModSeq, Uid};

/// Persisted as a unit alongside the message writes it describes, so a crash cannot leave the state
/// ahead of the messages it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub mailbox_id: MailboxId,
    pub account_id: AccountId,
    pub generation: Option<Generation>,
    pub uid_next: Option<Uid>,
    pub highest_mod_seq: Option<ModSeq>,
    pub last_full_sync_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// The `passes_since_sweep` of `SweepPolicy::decide`: a pass that read every flag, and a full
    /// enumeration, set it to 0; any other pass adds one.
    pub passes_since_flag_sweep: u32,
}

impl SyncState {
    pub fn never_synced(mailbox_id: MailboxId, account_id: AccountId) -> Self {
        Self {
            mailbox_id,
            account_id,
            generation: None,
            uid_next: None,
            highest_mod_seq: None,
            last_full_sync_at: None,
            last_seen_at: None,
            passes_since_flag_sweep: 0,
        }
    }

    /// Both halves are required: a mailbox selected once and then disconnected has a generation but
    /// no messages.
    pub fn has_synced(&self) -> bool {
        self.generation.is_some() && self.last_full_sync_at.is_some()
    }

    pub fn generation_changed(&self, observed: Generation) -> bool {
        matches!(self.generation, Some(known) if known != observed)
    }

    /// One decision from the whole state, so no caller reimplements the precedence between a
    /// `UIDVALIDITY` change and a `MODSEQ` comparison.
    pub fn plan(&self, status: &MailboxStatus) -> ResyncPlan {
        // Ordered by severity. A UIDVALIDITY change outranks everything: under
        // it the stored MODSEQ describes a UID space that no longer exists.
        if self.generation_changed(status.generation) {
            return ResyncPlan::Full(FullResyncReason::GenerationChanged);
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

    /// Advances `last_seen_at` but never `last_full_sync_at`: selecting is not synchronizing. A
    /// `UIDVALIDITY` change drops every
    /// cached counter, since a `MODSEQ` from the previous UID space is the corruption this type
    /// prevents.
    pub fn observe(&mut self, status: &MailboxStatus, at: DateTime<Utc>) {
        if self.generation_changed(status.generation) {
            self.highest_mod_seq = None;
            self.uid_next = None;
            self.last_full_sync_at = None;
            self.passes_since_flag_sweep = 0;
        }
        self.generation = Some(status.generation);
        if let Some(uid_next) = status.uid_next {
            self.uid_next = Some(uid_next);
        }
        if let Some(mod_seq) = status.highest_mod_seq {
            self.highest_mod_seq = Some(mod_seq);
        }
        self.last_seen_at = Some(at);
    }

    /// An enumeration reads every message's flags, so it counts as a sweep.
    pub fn complete_full_sync(&mut self, at: DateTime<Utc>) {
        self.last_full_sync_at = Some(at);
        self.last_seen_at = Some(at);
        self.passes_since_flag_sweep = 0;
    }

    /// Called after flags were reconciled and `observe` took `HIGHESTMODSEQ`, so a pass that failed
    /// part-way counts as nothing and is decided again.
    pub fn record_flag_pass(&mut self, swept: bool) {
        self.passes_since_flag_sweep = if swept {
            0
        } else {
            self.passes_since_flag_sweep.saturating_add(1)
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStatus {
    /// The mailbox's current naming generation. Always present for IMAP: a
    /// server that selects a mailbox must report it.
    pub generation: Generation,
    pub uid_next: Option<Uid>,
    pub highest_mod_seq: Option<ModSeq>,
}

impl MailboxStatus {
    pub fn new(generation: Generation) -> Self {
        Self {
            generation,
            uid_next: None,
            highest_mod_seq: None,
        }
    }

    pub fn with_uid_next(mut self, uid_next: Uid) -> Self {
        self.uid_next = Some(uid_next);
        self
    }

    pub fn with_highest_mod_seq(mut self, mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = Some(mod_seq);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResyncPlan {
    Full(FullResyncReason),
    Incremental { since: ModSeq },
    UpToDate,
}

impl ResyncPlan {
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full(_))
    }
}

/// Carried rather than a bool because the sync-status UI and bug reports need it: "your server
/// renumbered the mailbox" and "this mailbox is new" produce identical traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FullResyncReason {
    NeverSynced,
    GenerationChanged,
    /// One side does not support CONDSTORE, so "what changed" cannot be asked.
    NoModSeq,
    /// The server reported a `MODSEQ` below the one already seen, which the
    /// protocol says cannot happen — its store was rebuilt.
    ModSeqWentBackwards,
    /// Not reachable from `SyncState::plan`, which sees only protocol state: `resync` decides it
    /// from the local row count and the operation
    /// queue. It lets a mailbox recorded as synchronized without ever being enumerated leave that
    /// state without deleting the store.
    ShortOfExists,
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
        state.generation = Some(Generation::new(1_707_000_000));
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
            state.plan(&MailboxStatus::new(Generation::new(7))),
            ResyncPlan::Full(FullResyncReason::NeverSynced)
        );
    }

    #[test]
    fn selecting_a_mailbox_once_is_not_having_synced_it() {
        let mut state = SyncState::never_synced(MailboxId::new(1), AccountId::new(1));
        state.observe(&MailboxStatus::new(Generation::new(7)), at(9));

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
        let status = MailboxStatus::new(Generation::new(1_707_000_000))
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
        let status = MailboxStatus::new(Generation::new(1_707_000_000))
            .with_highest_mod_seq(ModSeq::new(90_210));

        assert_eq!(state.plan(&status), ResyncPlan::UpToDate);
        assert!(!state.plan(&status).is_full());
    }

    #[test]
    fn a_generation_change_outranks_a_matching_mod_seq() {
        let state = synced();
        let status = MailboxStatus::new(Generation::new(1_800_000_000))
            .with_highest_mod_seq(ModSeq::new(90_210));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::GenerationChanged)
        );
    }

    #[test]
    fn a_server_without_condstore_forces_a_full_pass() {
        let state = synced();
        let status = MailboxStatus::new(Generation::new(1_707_000_000));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::NoModSeq)
        );
    }

    #[test]
    fn a_mod_seq_that_went_backwards_forces_a_full_pass() {
        let state = synced();
        let status = MailboxStatus::new(Generation::new(1_707_000_000))
            .with_highest_mod_seq(ModSeq::new(90_000));

        assert_eq!(
            state.plan(&status),
            ResyncPlan::Full(FullResyncReason::ModSeqWentBackwards)
        );
    }

    #[test]
    fn observing_a_new_generation_drops_the_counters_that_belonged_to_the_old_one() {
        let mut state = synced();
        state.observe(&MailboxStatus::new(Generation::new(1_800_000_000)), at(10));

        assert_eq!(state.generation, Some(Generation::new(1_800_000_000)));
        assert_eq!(state.highest_mod_seq, None, "belonged to the old UID space");
        assert_eq!(state.uid_next, None, "belonged to the old UID space");
        assert!(!state.has_synced(), "the mailbox has to be re-enumerated");
        assert_eq!(state.last_seen_at, Some(at(10)));
    }

    #[test]
    fn observing_the_same_generation_advances_the_counters() {
        let mut state = synced();
        let status = MailboxStatus::new(Generation::new(1_707_000_000))
            .with_uid_next(Uid::new(4_500))
            .with_highest_mod_seq(ModSeq::new(90_300));
        state.observe(&status, at(10));

        assert_eq!(state.uid_next, Some(Uid::new(4_500)));
        assert_eq!(state.highest_mod_seq, Some(ModSeq::new(90_300)));
        assert!(state.has_synced(), "a completed sync is still completed");
        assert_eq!(state.last_full_sync_at, Some(at(9)), "not a full sync");
    }

    #[test]
    fn passes_are_counted_until_a_sweep_resets_them() {
        let mut state = synced();
        state.record_flag_pass(false);
        state.record_flag_pass(false);
        assert_eq!(state.passes_since_flag_sweep, 2);
        state.record_flag_pass(true);
        assert_eq!(state.passes_since_flag_sweep, 0);
    }

    #[test]
    fn a_full_sync_and_a_new_generation_both_restart_the_count() {
        let mut state = synced();
        state.record_flag_pass(false);
        state.complete_full_sync(at(10));
        assert_eq!(
            state.passes_since_flag_sweep, 0,
            "enumerating read the flags"
        );

        state.record_flag_pass(false);
        state.observe(&MailboxStatus::new(Generation::new(1_800_000_000)), at(11));
        assert_eq!(state.passes_since_flag_sweep, 0, "a new UID space");
    }

    #[test]
    fn a_status_that_omits_a_counter_does_not_erase_it() {
        let mut state = synced();
        state.observe(&MailboxStatus::new(Generation::new(1_707_000_000)), at(10));

        assert_eq!(
            state.highest_mod_seq,
            Some(ModSeq::new(90_210)),
            "the server did not say, which is not the same as saying none"
        );
        assert_eq!(state.uid_next, Some(Uid::new(4_412)));
    }
}
