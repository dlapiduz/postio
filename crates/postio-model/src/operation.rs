//! The mutation vocabulary: what a local-first action asks the server to do,
//! and what undoes it.
//!
//! # Why this is in the model
//!
//! Every mutating action in Postio is local-first: write SQLite, enqueue the
//! operation, emit the event, repaint — the UI never awaits the network
//! (`CLAUDE.md`). Three layers then have to speak the same vocabulary. The sync
//! engine drains the queue, the undo stack in `postio-core` replays inverses,
//! and the UI names the pending action in its undo toast. Only `postio-model`
//! is visible to all three, so the vocabulary lives here; the queue table that
//! stores it lives in `postio-storage`, and the code that *executes* an
//! operation against a server lives in `postio-sync`.
//!
//! # Undo is not a second code path
//!
//! [`Operation::inverse`] returns another [`Operation`]. Undo enqueues it
//! exactly the way the original action was enqueued, so there is one drain
//! path, one retry policy and one conflict resolution — not a parallel
//! "un-archive" implementation that will drift. Operations that genuinely
//! cannot be taken back return `None`, and the UI must not offer undo for
//! them; see [`Operation::inverse`] for which and why.

use serde::{Deserialize, Serialize};

use crate::flag::FlagSet;
use crate::ids::{
    AccountId, BlobId, CrossAccountMoveId, DraftId, MailboxId, MessageId, OperationId, ThreadId,
    RemoteId, Uid, UidValidity,
};

/// What an operation acts on.
///
/// Stored as the `(target_kind, target_id)` pair the schema constrains, so the
/// queue can answer "does this message still have operations in flight?"
/// without parsing the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum OperationTarget {
    /// One message.
    Message(MessageId),
    /// A whole conversation.
    Thread(ThreadId),
    /// A folder.
    Mailbox(MailboxId),
    /// A draft being composed or sent.
    Draft(DraftId),
    /// The account itself.
    Account(AccountId),
}

impl OperationTarget {
    /// The stored spelling of the kind, which the schema's `CHECK` enforces.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Thread(_) => "thread",
            Self::Mailbox(_) => "mailbox",
            Self::Draft(_) => "draft",
            Self::Account(_) => "account",
        }
    }

    /// The row id being targeted.
    pub fn id(self) -> i64 {
        match self {
            Self::Message(id) => id.get(),
            Self::Thread(id) => id.get(),
            Self::Mailbox(id) => id.get(),
            Self::Draft(id) => id.get(),
            Self::Account(id) => id.get(),
        }
    }

    /// Rebuilds a target from the pair of columns it was stored as.
    ///
    /// Returns `None` for a kind this build does not know, so a row written by
    /// a newer Postio is reported rather than guessed at.
    pub fn from_parts(kind: &str, id: i64) -> Option<Self> {
        match kind {
            "message" => Some(Self::Message(MessageId::new(id))),
            "thread" => Some(Self::Thread(ThreadId::new(id))),
            "mailbox" => Some(Self::Mailbox(MailboxId::new(id))),
            "draft" => Some(Self::Draft(DraftId::new(id))),
            "account" => Some(Self::Account(AccountId::new(id))),
            _ => None,
        }
    }
}

/// A contiguous run of queue rows, written by one bulk enqueue.
///
/// # Why a range is the handle a bulk undo needs
///
/// Archiving an 81,717-message mailbox writes one queue row per message —
/// each one carries the UID the drainer needs, so they cannot be folded into
/// one — but it must not put 81,717 ids anywhere a human-scale data structure
/// can hold them. The undo entry therefore names them the way the queue
/// already numbered them: everything between two ids.
///
/// Both ends are inclusive, and the range is exact rather than merely
/// contiguous — the writer takes `first` from the highest id present *before*
/// its statement and `last` from the id its statement ended on, so a row
/// another writer slipped in cannot widen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRange {
    /// The first row of the run.
    pub first: OperationId,
    /// The last row of the run, inclusive.
    pub last: OperationId,
}

impl OperationRange {
    /// The run from `first` through `last`, both included.
    pub fn new(first: OperationId, last: OperationId) -> Self {
        Self { first, last }
    }

    /// Whether the run names no rows at all.
    ///
    /// A bulk enqueue over an empty mailbox writes nothing, and `last` then
    /// sits below `first`.
    pub fn is_empty(self) -> bool {
        self.last.get() < self.first.get()
    }
}

/// Where a queued operation is in its life cycle.
///
/// Stored as the `as_str` spelling the schema's `CHECK` constrains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OperationState {
    /// Waiting to be drained. The state everything is enqueued in.
    #[default]
    Pending,
    /// Handed to the server; the outcome is not known yet.
    ///
    /// A row left here by a crash is *not* known to have failed — the server
    /// may well have applied it — so the drainer returns it to
    /// [`OperationState::Pending`] on start and relies on operations being
    /// idempotent rather than guessing.
    InFlight,
    /// Applied on the server. Kept briefly so undo can still find it.
    Done,
    /// Given up on. Only the user can clear it.
    Failed,
}

impl OperationState {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InFlight => "in_flight",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// The inverse of [`OperationState::as_str`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pending" => Some(Self::Pending),
            "in_flight" => Some(Self::InFlight),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// Whether the drainer will still pick this row up.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// A mutation waiting to be replayed against the server.
///
/// The payload of a queue row. Serialized as an internally tagged JSON object
/// whose tag matches [`Operation::op_type`], so a queue row can be read by eye
/// during a bug report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Add flags to the target.
    SetFlags {
        /// The flags to add.
        flags: FlagSet,
    },
    /// Remove flags from the target.
    ClearFlags {
        /// The flags to remove.
        flags: FlagSet,
    },
    /// Move the target between mailboxes. Archive is a move.
    Move {
        /// Where it is now.
        from: MailboxId,
        /// Where it should end up.
        to: MailboxId,
    },
    /// Move the target to the account's trash.
    ///
    /// Distinct from [`Operation::Move`] with the trash as its destination
    /// because the two mean different things to the user, and the undo toast
    /// says so — but the inverse is the same move, run backwards.
    Delete {
        /// Where it is now.
        from: MailboxId,
        /// The account's trash mailbox.
        trash: MailboxId,
    },
    /// Permanently remove everything marked `\Deleted` from a mailbox.
    Expunge {
        /// The mailbox to expunge.
        mailbox: MailboxId,
    },
    /// Upload a message into a mailbox, from a blob already in the store.
    Append {
        /// Where it goes.
        mailbox: MailboxId,
        /// The raw RFC 5322 bytes, content-addressed.
        blob: BlobId,
        /// Flags to set on arrival.
        flags: FlagSet,
    },
    /// Hand a draft to the SMTP transport.
    Send {
        /// The draft to send.
        draft: DraftId,
    },
    /// Put the draft named by the target in the account's Drafts mailbox,
    /// replacing whatever copy of it is already there.
    ///
    /// Deliberately carries no bytes. A draft is autosaved as the user types,
    /// and building its RFC 5322 form means reading every attachment out of
    /// the blob store — so the bytes are built once, when this drains, from
    /// whatever the draft says *then*. That is also what makes two of these
    /// fold into one: the later save does not describe different work, it
    /// describes the same work against a newer draft.
    SaveDraft {
        /// The account's Drafts mailbox.
        mailbox: MailboxId,
    },
    /// Remove a draft's copy from the account's Drafts mailbox.
    ///
    /// This one *does* name its message, because by the time it drains the
    /// local draft row is gone — discarding a draft is local-first like
    /// everything else, so the row disappears at once and the server catches
    /// up later. The identity is the backend's own [`RemoteId`] (#543): for
    /// IMAP it packs the generation, so a copy uploaded under a renumbered
    /// mailbox can never be confused with whatever message holds that
    /// number now. (Queued rows from before #543 are rewritten by
    /// migration 0024.)
    DiscardDraft {
        /// The account's Drafts mailbox.
        mailbox: MailboxId,
        /// The server copy to remove.
        remote_id: RemoteId,
    },
    /// Phases 1–2 of a cross-account move (#188, ADR 0005 Q9): append the
    /// message to the target account's mailbox and confirm it arrived.
    ///
    /// Runs on the **target** account's queue. Everything it needs — the
    /// source blob, the target mailbox, the phase — lives on the saga row,
    /// which is the state the two queues share instead of a transaction.
    CrossAccountCopy {
        /// The saga this operation advances.
        saga: CrossAccountMoveId,
    },
    /// Phase 3 of a cross-account move: `\Deleted` + expunge on the source.
    ///
    /// Runs on the **source** account's queue, and the drainer refuses it
    /// until the saga row says the target copy is confirmed — the ordering
    /// that makes the only possible failure a duplicate, never a loss.
    CrossAccountRemove {
        /// The saga this operation completes.
        saga: CrossAccountMoveId,
    },
}

impl Operation {
    /// The stored `op_type` spelling, matching the serialized tag.
    pub fn op_type(&self) -> &'static str {
        match self {
            Self::SetFlags { .. } => "set_flags",
            Self::ClearFlags { .. } => "clear_flags",
            Self::Move { .. } => "move",
            Self::Delete { .. } => "delete",
            Self::Expunge { .. } => "expunge",
            Self::Append { .. } => "append",
            Self::Send { .. } => "send",
            Self::SaveDraft { .. } => "save_draft",
            Self::DiscardDraft { .. } => "discard_draft",
            Self::CrossAccountCopy { .. } => "cross_account_copy",
            Self::CrossAccountRemove { .. } => "cross_account_remove",
        }
    }

    /// The operation that undoes this one, if any.
    ///
    /// # What has no inverse, and why
    ///
    /// * [`Operation::Expunge`] destroys mail on the server. Nothing local can
    ///   bring it back, and offering an undo that silently did nothing would be
    ///   worse than offering none.
    /// * [`Operation::Append`] and [`Operation::Send`] produce a message whose
    ///   server identity does not exist yet at enqueue time, so there is
    ///   nothing to name in an inverse. Undoing a send is a *cancel* against
    ///   the queue — dropping the row before it drains — not a compensating
    ///   operation, and that is the undo the composer offers.
    /// * [`Operation::SaveDraft`] would have to restore the text the draft had
    ///   before, which is not kept — the composer's undo is over the text
    ///   itself, and the next save carries it to the server like any other
    ///   edit. [`Operation::DiscardDraft`] destroys the server's only copy,
    ///   the same way [`Operation::Expunge`] does.
    /// * The cross-account phases are steps of a saga, not actions: the undo
    ///   of a cross-account *move* is the inverse saga (ADR 0005 Q9), driven
    ///   from the saga table where the confirmed target UID lives — never a
    ///   per-operation inverse, which could delete before anything copied.
    ///
    /// Everything else round-trips: applying an operation and then its inverse
    /// leaves the mailbox as it was.
    pub fn inverse(&self) -> Option<Self> {
        match self {
            Self::SetFlags { flags } => Some(Self::ClearFlags {
                flags: flags.clone(),
            }),
            Self::ClearFlags { flags } => Some(Self::SetFlags {
                flags: flags.clone(),
            }),
            Self::Move { from, to } => Some(Self::Move {
                from: *to,
                to: *from,
            }),
            Self::Delete { from, trash } => Some(Self::Move {
                from: *trash,
                to: *from,
            }),
            Self::Expunge { .. }
            | Self::Append { .. }
            | Self::Send { .. }
            | Self::SaveDraft { .. }
            | Self::DiscardDraft { .. }
            | Self::CrossAccountCopy { .. }
            | Self::CrossAccountRemove { .. } => None,
        }
    }

    /// Whether the user can be offered an undo for this.
    pub fn is_reversible(&self) -> bool {
        self.inverse().is_some()
    }

    /// The mailbox this operation touches, when it names one.
    ///
    /// Denormalized into its own column so the drainer can group a batch by
    /// mailbox and select it once, rather than re-selecting per row.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            Self::Move { from, .. } | Self::Delete { from, .. } => Some(*from),
            Self::Expunge { mailbox }
            | Self::Append { mailbox, .. }
            | Self::SaveDraft { mailbox }
            | Self::DiscardDraft { mailbox, .. } => Some(*mailbox),
            Self::SetFlags { .. }
            | Self::ClearFlags { .. }
            | Self::Send { .. }
            | Self::CrossAccountCopy { .. }
            | Self::CrossAccountRemove { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flag::Flag;

    fn flags(raw: &str) -> FlagSet {
        raw.split_whitespace().map(Flag::parse).collect()
    }

    fn every_operation() -> Vec<Operation> {
        vec![
            Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            Operation::ClearFlags {
                flags: flags("\\Seen \\Flagged"),
            },
            Operation::Move {
                from: MailboxId::new(1),
                to: MailboxId::new(2),
            },
            Operation::Delete {
                from: MailboxId::new(1),
                trash: MailboxId::new(9),
            },
            Operation::Expunge {
                mailbox: MailboxId::new(9),
            },
            Operation::Append {
                mailbox: MailboxId::new(3),
                blob: BlobId::new("abc123"),
                flags: flags("\\Seen"),
            },
            Operation::Send {
                draft: DraftId::new(4),
            },
            Operation::SaveDraft {
                mailbox: MailboxId::new(5),
            },
            Operation::DiscardDraft {
                mailbox: MailboxId::new(5),
                remote_id: RemoteId::new("12:77"),
            },
            Operation::CrossAccountCopy {
                saga: CrossAccountMoveId::new(3),
            },
            Operation::CrossAccountRemove {
                saga: CrossAccountMoveId::new(3),
            },
        ]
    }

    #[test]
    fn every_operation_has_a_decided_inverse() {
        for operation in every_operation() {
            // The point is that no variant is left undecided: `inverse` is
            // exhaustive, so adding a variant without deciding fails to compile
            // and this test names what was decided.
            let expected = match &operation {
                Operation::SetFlags { .. }
                | Operation::ClearFlags { .. }
                | Operation::Move { .. }
                | Operation::Delete { .. } => true,
                Operation::Expunge { .. }
                | Operation::Append { .. }
                | Operation::Send { .. }
                | Operation::SaveDraft { .. }
                | Operation::DiscardDraft { .. }
                // Saga phases: the undo is the inverse *saga*, never a
                // per-operation inverse — see `inverse`'s own docs.
                | Operation::CrossAccountCopy { .. }
                | Operation::CrossAccountRemove { .. } => false,
            };
            assert_eq!(
                operation.is_reversible(),
                expected,
                "{} decided the wrong way",
                operation.op_type()
            );
        }
    }

    #[test]
    fn flag_operations_invert_each_other() {
        let set = Operation::SetFlags {
            flags: flags("\\Seen"),
        };
        let clear = set.inverse().expect("reversible");

        assert_eq!(
            clear,
            Operation::ClearFlags {
                flags: flags("\\Seen")
            }
        );
        assert_eq!(clear.inverse().as_ref(), Some(&set), "and back again");
    }

    #[test]
    fn a_move_inverts_by_swapping_its_ends() {
        let archive = Operation::Move {
            from: MailboxId::new(1),
            to: MailboxId::new(2),
        };
        let unarchive = archive.inverse().expect("reversible");

        assert_eq!(
            unarchive,
            Operation::Move {
                from: MailboxId::new(2),
                to: MailboxId::new(1)
            }
        );
        assert_eq!(unarchive.inverse().as_ref(), Some(&archive));
    }

    #[test]
    fn a_delete_inverts_to_a_move_back_out_of_the_trash() {
        let delete = Operation::Delete {
            from: MailboxId::new(1),
            trash: MailboxId::new(9),
        };

        assert_eq!(
            delete.inverse(),
            Some(Operation::Move {
                from: MailboxId::new(9),
                to: MailboxId::new(1)
            }),
            "undo puts the message back where the user had it"
        );
    }

    #[test]
    fn an_expunge_is_not_offered_an_undo() {
        let expunge = Operation::Expunge {
            mailbox: MailboxId::new(9),
        };

        assert_eq!(expunge.inverse(), None);
        assert!(!expunge.is_reversible());
    }

    #[test]
    fn appending_and_sending_are_cancelled_rather_than_inverted() {
        for operation in [
            Operation::Append {
                mailbox: MailboxId::new(3),
                blob: BlobId::new("abc123"),
                flags: FlagSet::new(),
            },
            Operation::Send {
                draft: DraftId::new(4),
            },
        ] {
            assert_eq!(
                operation.inverse(),
                None,
                "{} names no message that exists yet",
                operation.op_type()
            );
        }
    }

    #[test]
    fn every_op_type_is_distinct_and_matches_the_serialized_tag() {
        let mut seen = std::collections::BTreeSet::new();
        for operation in every_operation() {
            assert!(
                seen.insert(operation.op_type()),
                "duplicate op_type {}",
                operation.op_type()
            );
            let encoded = serde_json::to_value(&operation).expect("encode");
            assert_eq!(
                encoded.get("op").and_then(|tag| tag.as_str()),
                Some(operation.op_type()),
                "the column and the payload must agree"
            );
        }
    }

    #[test]
    fn every_operation_round_trips_through_json() {
        for operation in every_operation() {
            let encoded = serde_json::to_string(&operation).expect("encode");
            let decoded: Operation = serde_json::from_str(&encoded).expect("decode");
            assert_eq!(decoded, operation, "{encoded}");
        }
    }

    #[test]
    fn every_target_kind_round_trips_through_its_stored_columns() {
        for target in [
            OperationTarget::Message(MessageId::new(1)),
            OperationTarget::Thread(ThreadId::new(2)),
            OperationTarget::Mailbox(MailboxId::new(3)),
            OperationTarget::Draft(DraftId::new(4)),
            OperationTarget::Account(AccountId::new(5)),
        ] {
            assert_eq!(
                OperationTarget::from_parts(target.kind(), target.id()),
                Some(target)
            );
        }
        assert_eq!(OperationTarget::from_parts("nonsense", 1), None);
    }

    #[test]
    fn every_state_round_trips_through_its_stored_spelling() {
        for state in [
            OperationState::Pending,
            OperationState::InFlight,
            OperationState::Done,
            OperationState::Failed,
        ] {
            assert_eq!(OperationState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(
            OperationState::from_name("Pending"),
            None,
            "spelling is exact"
        );
        assert_eq!(OperationState::from_name("nonsense"), None);
        assert!(!OperationState::default().is_settled());
        assert!(OperationState::Failed.is_settled());
    }

    #[test]
    fn an_operation_names_the_mailbox_the_drainer_has_to_select() {
        assert_eq!(
            Operation::Move {
                from: MailboxId::new(1),
                to: MailboxId::new(2)
            }
            .mailbox(),
            Some(MailboxId::new(1)),
            "the source: that is the mailbox that has to be selected"
        );
        assert_eq!(
            Operation::SetFlags {
                flags: flags("\\Seen")
            }
            .mailbox(),
            None,
            "the target's own mailbox, which the row already carries"
        );
        assert_eq!(
            Operation::SaveDraft {
                mailbox: MailboxId::new(5)
            }
            .mailbox(),
            Some(MailboxId::new(5)),
            "a draft is filed in the account's Drafts mailbox"
        );
    }

    #[test]
    fn the_draft_operations_are_not_offered_an_undo() {
        for operation in [
            Operation::SaveDraft {
                mailbox: MailboxId::new(5),
            },
            Operation::DiscardDraft {
                mailbox: MailboxId::new(5),
                remote_id: RemoteId::new("12:77"),
            },
        ] {
            assert_eq!(
                operation.inverse(),
                None,
                "{} has nothing to restore: the text it replaced is not kept",
                operation.op_type()
            );
        }
    }

    #[test]
    fn discarding_a_draft_names_its_server_copy_by_identity() {
        // The whole reason this operation carries its own identity rather than
        // reading it back from a draft row: by the time it drains, that row is
        // gone. The id is the backend's own spelling, so a renumbered mailbox
        // makes it stale rather than silently naming somebody else's message.
        let discard = Operation::DiscardDraft {
            mailbox: MailboxId::new(5),
            remote_id: RemoteId::new("12:77"),
        };
        let encoded = serde_json::to_value(&discard).expect("encode");

        assert_eq!(
            encoded.get("remote_id").and_then(|id| id.as_str()),
            Some("12:77")
        );
    }
}
