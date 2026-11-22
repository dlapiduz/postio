//! What a local-first action asks the server to do, and what undoes it; the sync engine, the undo
//! stack and the undo toast all speak it.
//! `Operation::inverse` returns another `Operation`, enqueued like the original: one drain path,
//! retry policy and conflict resolution.
//! Operations that cannot be taken back return `None`, and the UI must not offer undo for them.

use serde::{Deserialize, Serialize};

use crate::flag::FlagSet;
use crate::ids::{
    AccountId, BlobId, CrossAccountMoveId, DraftId, MailboxId, MessageId, OperationId, RemoteId,
    ThreadId,
};

/// Stored as the `(target_kind, target_id)` pair, so the queue can answer "does this message have
/// operations in flight?" without parsing the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum OperationTarget {
    Message(MessageId),
    Thread(ThreadId),
    Mailbox(MailboxId),
    Draft(DraftId),
    Account(AccountId),
}

impl OperationTarget {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Thread(_) => "thread",
            Self::Mailbox(_) => "mailbox",
            Self::Draft(_) => "draft",
            Self::Account(_) => "account",
        }
    }

    pub fn id(self) -> i64 {
        match self {
            Self::Message(id) => id.get(),
            Self::Thread(id) => id.get(),
            Self::Mailbox(id) => id.get(),
            Self::Draft(id) => id.get(),
            Self::Account(id) => id.get(),
        }
    }

    /// `None` for a kind this build does not know, so a row written by a newer Postio is reported,
    /// not guessed at.
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

/// The undo handle for a bulk enqueue: one row per message (each carries its UID) but never one id
/// each in a human-scale
/// structure. Inclusive and exact: `first` is the highest id before the statement and `last` where
/// it ended, so another writer's row cannot widen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRange {
    pub first: OperationId,
    pub last: OperationId,
}

impl OperationRange {
    pub fn new(first: OperationId, last: OperationId) -> Self {
        Self { first, last }
    }

    /// An enqueue over an empty mailbox writes nothing, and `last` then sits below `first`.
    pub fn is_empty(self) -> bool {
        self.last.get() < self.first.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OperationState {
    #[default]
    Pending,
    /// A row left here by a crash is not known to have failed, so the drainer returns it to
    /// `Pending` on start and relies on idempotent operations.
    InFlight,
    /// Applied on the server. Kept briefly so undo can still find it.
    Done,
    /// Given up on. Only the user can clear it.
    Failed,
}

impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InFlight => "in_flight",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pending" => Some(Self::Pending),
            "in_flight" => Some(Self::InFlight),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_settled(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// Internally tagged JSON whose tag matches `Operation::op_type`, so a queue row can be read by eye
/// in a bug report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    SetFlags {
        flags: FlagSet,
    },
    ClearFlags {
        flags: FlagSet,
    },
    Move {
        from: MailboxId,
        to: MailboxId,
    },
    /// Distinct from `Move` to trash because the undo toast says so; the inverse is the same move
    /// backwards.
    Delete {
        from: MailboxId,
        trash: MailboxId,
    },
    Expunge {
        mailbox: MailboxId,
    },
    /// Marks it `\Deleted` and expunges exactly it, never the mailbox's other marked mail (#1932).
    Purge {
        mailbox: MailboxId,
    },
    Append {
        mailbox: MailboxId,
        blob: BlobId,
        flags: FlagSet,
    },
    Send {
        draft: DraftId,
    },
    /// Carries no bytes: a draft is autosaved as the user types and building its RFC 5322 form
    /// reads every attachment from the
    /// blob store, so the bytes are built once, when this drains, from the draft as it then is.
    /// That is also what lets two of these fold into one.
    SaveDraft {
        mailbox: MailboxId,
    },
    /// Names its message because the local draft row is gone by the time it drains. The identity is
    /// the backend's `RemoteId`
    /// (#543), which for IMAP packs the generation, so a copy uploaded under a renumbered mailbox
    /// cannot be confused with another message.
    DiscardDraft {
        mailbox: MailboxId,
        remote_id: RemoteId,
    },
    /// Phases 1-2 of a cross-account move (#188, ADR 0005 Q9). Runs on the target account's queue;
    /// the source blob,
    /// target mailbox and phase live on the saga row, the state the two queues share instead of a
    /// transaction.
    CrossAccountCopy {
        saga: CrossAccountMoveId,
    },
    /// Runs on the source account's queue, and the drainer refuses it until the saga row says the
    /// target copy is confirmed:
    /// the only possible failure is a duplicate, never a loss.
    CrossAccountRemove {
        saga: CrossAccountMoveId,
    },
}

impl Operation {
    pub fn op_type(&self) -> &'static str {
        match self {
            Self::SetFlags { .. } => "set_flags",
            Self::ClearFlags { .. } => "clear_flags",
            Self::Move { .. } => "move",
            Self::Delete { .. } => "delete",
            Self::Expunge { .. } => "expunge",
            Self::Purge { .. } => "purge",
            Self::Append { .. } => "append",
            Self::Send { .. } => "send",
            Self::SaveDraft { .. } => "save_draft",
            Self::DiscardDraft { .. } => "discard_draft",
            Self::CrossAccountCopy { .. } => "cross_account_copy",
            Self::CrossAccountRemove { .. } => "cross_account_remove",
        }
    }

    /// No inverse: `Expunge` destroys mail, and an undo that silently did nothing is worse than
    /// none. `Append` and `Send` have no
    /// server identity at enqueue time; undoing a send cancels the queued row. `SaveDraft` would
    /// need the prior text, which is not kept;
    /// `DiscardDraft` destroys the server's only copy. The cross-account phases are saga steps:
    /// undo is the inverse saga (ADR 0005 Q9),
    /// never a per-operation inverse, which could delete before anything copied.
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
            | Self::Purge { .. }
            | Self::Append { .. }
            | Self::Send { .. }
            | Self::SaveDraft { .. }
            | Self::DiscardDraft { .. }
            | Self::CrossAccountCopy { .. }
            | Self::CrossAccountRemove { .. } => None,
        }
    }

    pub fn is_reversible(&self) -> bool {
        self.inverse().is_some()
    }

    /// Denormalized into its own column so the drainer can group a batch by mailbox and select
    /// once.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            Self::Move { from, .. } | Self::Delete { from, .. } => Some(*from),
            Self::Expunge { mailbox }
            | Self::Purge { mailbox }
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
            Operation::Purge {
                mailbox: MailboxId::new(9),
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
            let expected = match &operation {
                Operation::SetFlags { .. }
                | Operation::ClearFlags { .. }
                | Operation::Move { .. }
                | Operation::Delete { .. } => true,
                Operation::Expunge { .. }
                | Operation::Purge { .. }
                | Operation::Append { .. }
                | Operation::Send { .. }
                | Operation::SaveDraft { .. }
                | Operation::DiscardDraft { .. }
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
