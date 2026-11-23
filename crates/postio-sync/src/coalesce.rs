//! Folds a queue batch to the operations the server needs: flag changes merge, moves chain, draft
//! saves merge.
//! Conservative: only same-target operations fold, only when exactly equivalent, since a wrong fold
//! loses a mutation.
//! `Append`, `Send` and `Expunge` never fold. Each step keeps the position of its earliest row.

use postio_model::{FlagSet, Operation, OperationId, OperationTarget};
use postio_storage::repository::QueuedOperation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub operation: Operation,
    pub target: OperationTarget,
    pub rows: Vec<OperationId>,
}

impl Step {
    /// The earliest queue row behind this step: the row whose source snapshot predates any local
    /// nulling (#289).
    pub(crate) fn head(&self) -> OperationId {
        self.rows[0]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Rows that cancelled out entirely: still settled, but need no server round trip.
    pub obsolete: Vec<OperationId>,
}

impl Plan {
    pub fn rows(&self) -> usize {
        self.steps.iter().map(|step| step.rows.len()).sum::<usize>() + self.obsolete.len()
    }

    pub fn saved(&self) -> usize {
        self.rows() - self.steps.len()
    }
}

/// Folds a batch, which must be in queue order.
pub fn coalesce(batch: &[QueuedOperation]) -> Plan {
    let mut folds: Vec<Fold> = Vec::new();

    for queued in batch {
        match folds
            .iter_mut()
            .find(|fold| fold.target == queued.target && fold.open)
        {
            Some(fold) => fold.push(queued),
            None => folds.push(Fold::new(queued)),
        }
    }

    let mut plan = Plan::default();
    for fold in folds {
        let (steps, obsolete) = fold.finish();
        plan.steps.extend(steps);
        plan.obsolete.extend(obsolete);
    }

    plan.steps.sort_by_key(Step::head);
    plan.obsolete.sort_unstable();
    plan
}

#[derive(Debug)]
struct Fold {
    target: OperationTarget,
    steps: Vec<Step>,
    obsolete: Vec<OperationId>,
    open: bool,
}

impl Fold {
    fn new(queued: &QueuedOperation) -> Self {
        let mut fold = Self {
            target: queued.target,
            steps: Vec::new(),
            obsolete: Vec::new(),
            open: true,
        };
        fold.push(queued);
        fold
    }

    fn push(&mut self, queued: &QueuedOperation) {
        if !foldable(&queued.operation) {
            // Not idempotent: nothing before it may be rewritten in its light, nothing after pulled
            // back across it.
            self.steps.push(Step {
                operation: queued.operation.clone(),
                target: queued.target,
                rows: vec![queued.id],
            });
            self.open = false;
            return;
        }

        let Some(previous) = self.steps.last_mut() else {
            self.steps.push(Step {
                operation: queued.operation.clone(),
                target: queued.target,
                rows: vec![queued.id],
            });
            return;
        };

        match merge(&previous.operation, &queued.operation) {
            Merged::Into(operation) => {
                previous.operation = operation;
                previous.rows.push(queued.id);
            }
            Merged::Cancelled => {
                let mut rows = std::mem::take(&mut previous.rows);
                rows.push(queued.id);
                self.steps.pop();
                self.obsolete.extend(rows);
            }
            Merged::No => {
                self.steps.push(Step {
                    operation: queued.operation.clone(),
                    target: queued.target,
                    rows: vec![queued.id],
                });
            }
        }
    }

    fn finish(self) -> (Vec<Step>, Vec<OperationId>) {
        (self.steps, self.obsolete)
    }
}

fn foldable(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::SetFlags { .. }
            | Operation::ClearFlags { .. }
            | Operation::Move { .. }
            | Operation::Delete { .. }
            | Operation::SaveDraft { .. }
            | Operation::DiscardDraft { .. }
    )
}

enum Merged {
    Into(Operation),
    Cancelled,
    No,
}

fn merge(earlier: &Operation, later: &Operation) -> Merged {
    use Operation::*;

    match (earlier, later) {
        (SetFlags { flags: first }, SetFlags { flags: second }) => Merged::Into(SetFlags {
            flags: union(first, second),
        }),
        (ClearFlags { flags: first }, ClearFlags { flags: second }) => Merged::Into(ClearFlags {
            flags: union(first, second),
        }),

        // Opposite directions cancel on the overlap; what is left of the earlier change still has
        // to be sent.
        (SetFlags { flags: set }, ClearFlags { flags: cleared })
        | (ClearFlags { flags: set }, SetFlags { flags: cleared })
            if difference(set, cleared).is_empty() =>
        {
            if difference(cleared, set).is_empty() {
                Merged::Cancelled
            } else {
                Merged::Into(later.clone())
            }
        }

        (Move { from, to }, Move { from: via, to: end }) if to == via => {
            if from == end {
                Merged::Cancelled
            } else {
                Merged::Into(Move {
                    from: *from,
                    to: *end,
                })
            }
        }
        // `from` must be where the message started, or the drainer would select a mailbox it left.
        (Move { from, to }, Delete { from: via, trash }) if to == via => Merged::Into(Delete {
            from: *from,
            trash: *trash,
        }),
        (Delete { from, trash }, Move { from: via, to: end }) if trash == via => {
            if from == end {
                Merged::Cancelled
            } else {
                Merged::Into(Move {
                    from: *trash,
                    to: *end,
                })
            }
        }

        // Saves carry no text (bytes are built when the step drains), so autosave's many saves are
        // one piece of work.
        (SaveDraft { mailbox: first }, SaveDraft { mailbox: second }) if first == second => {
            Merged::Into(later.clone())
        }
        // A discard subsumes an undrained save: uploading a copy only to remove it is two wasted
        // round trips.
        (
            SaveDraft { mailbox: first },
            DiscardDraft {
                mailbox: second, ..
            },
        ) if first == second => Merged::Into(later.clone()),

        _ => Merged::No,
    }
}

fn union(first: &FlagSet, second: &FlagSet) -> FlagSet {
    first.iter().chain(second.iter()).cloned().collect()
}

fn difference(from: &FlagSet, remove: &FlagSet) -> FlagSet {
    from.iter()
        .filter(|flag| !remove.contains(flag))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use postio_model::{AccountId, Flag, MailboxId, MessageId, OperationState};

    use super::*;

    fn flags(raw: &str) -> FlagSet {
        raw.split_whitespace().map(Flag::parse).collect()
    }

    fn message(id: i64) -> OperationTarget {
        OperationTarget::Message(MessageId::new(id))
    }

    fn draft(id: i64) -> OperationTarget {
        OperationTarget::Draft(postio_model::ids::DraftId::new(id))
    }

    fn row(id: i64, target: OperationTarget, operation: Operation) -> QueuedOperation {
        let at = Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap();
        QueuedOperation {
            id: OperationId::new(id),
            account_id: AccountId::new(1),
            target,
            inverse: operation.inverse(),
            operation,
            mailbox_id: None,
            state: OperationState::Pending,
            attempts: 0,
            last_error: None,
            next_attempt_at: None,
            created_at: at,
            updated_at: at,
            source_remote_id: None,
        }
    }

    fn inbox() -> MailboxId {
        MailboxId::new(1)
    }
    fn archive() -> MailboxId {
        MailboxId::new(2)
    }
    fn trash() -> MailboxId {
        MailboxId::new(3)
    }

    fn set(raw: &str) -> Operation {
        Operation::SetFlags { flags: flags(raw) }
    }
    fn clear(raw: &str) -> Operation {
        Operation::ClearFlags { flags: flags(raw) }
    }
    fn moved(from: MailboxId, to: MailboxId) -> Operation {
        Operation::Move { from, to }
    }

    #[test]
    fn two_flag_sets_on_one_message_become_one() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Seen")),
            row(2, message(1), set("\\Flagged")),
        ]);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].operation, set("\\Seen \\Flagged"));
        assert_eq!(
            plan.steps[0].rows,
            vec![OperationId::new(1), OperationId::new(2)],
            "both rows are settled by the one round trip"
        );
        assert_eq!(plan.saved(), 1);
    }

    #[test]
    fn flagging_and_unflagging_the_same_message_costs_the_server_nothing() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Flagged")),
            row(2, message(1), clear("\\Flagged")),
        ]);

        assert!(plan.steps.is_empty());
        assert_eq!(
            plan.obsolete,
            vec![OperationId::new(1), OperationId::new(2)],
            "settled without being sent, not dropped"
        );
        assert_eq!(plan.rows(), 2, "every row is still accounted for");
    }

    #[test]
    fn only_the_overlap_cancels() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Seen \\Flagged")),
            row(2, message(1), clear("\\Flagged")),
        ]);

        assert_eq!(plan.steps.len(), 2, "the \\Seen still has to be sent");
        assert_eq!(plan.steps[0].operation, set("\\Seen \\Flagged"));
        assert_eq!(plan.steps[1].operation, clear("\\Flagged"));
        assert!(plan.obsolete.is_empty());
    }

    #[test]
    fn a_clear_fully_covered_by_the_set_before_it_replaces_it() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Flagged")),
            row(2, message(1), clear("\\Seen \\Flagged")),
        ]);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.steps[0].operation,
            clear("\\Seen \\Flagged"),
            "the set is subsumed; clearing both is the whole story"
        );
        assert_eq!(plan.steps[0].rows.len(), 2);
    }

    #[test]
    fn flags_on_different_messages_never_fold_together() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Seen")),
            row(2, message(2), set("\\Seen")),
        ]);

        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].target, message(1));
        assert_eq!(plan.steps[1].target, message(2));
    }

    #[test]
    fn a_chain_of_moves_becomes_one_move() {
        let plan = coalesce(&[
            row(1, message(1), moved(inbox(), archive())),
            row(2, message(1), moved(archive(), trash())),
        ]);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].operation, moved(inbox(), trash()));
    }

    #[test]
    fn archive_then_undo_never_reaches_the_server() {
        let plan = coalesce(&[
            row(1, message(1), moved(inbox(), archive())),
            row(2, message(1), moved(archive(), inbox())),
        ]);

        assert!(plan.steps.is_empty());
        assert_eq!(plan.obsolete.len(), 2);
    }

    #[test]
    fn a_move_then_a_delete_deletes_from_where_the_message_started() {
        let plan = coalesce(&[
            row(1, message(1), moved(inbox(), archive())),
            row(
                2,
                message(1),
                Operation::Delete {
                    from: archive(),
                    trash: trash(),
                },
            ),
        ]);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.steps[0].operation,
            Operation::Delete {
                from: inbox(),
                trash: trash()
            },
            "the drainer would otherwise select a mailbox the message has left"
        );
    }

    #[test]
    fn deleting_then_restoring_cancels() {
        let plan = coalesce(&[
            row(
                1,
                message(1),
                Operation::Delete {
                    from: inbox(),
                    trash: trash(),
                },
            ),
            row(2, message(1), moved(trash(), inbox())),
        ]);

        assert!(plan.steps.is_empty());
        assert_eq!(plan.obsolete.len(), 2);
    }

    #[test]
    fn moves_that_do_not_meet_stay_separate() {
        let plan = coalesce(&[
            row(1, message(1), moved(inbox(), archive())),
            row(2, message(1), moved(trash(), inbox())),
        ]);

        assert_eq!(
            plan.steps.len(),
            2,
            "chaining these would invent a move the user never made"
        );
    }

    #[test]
    fn sends_and_appends_are_never_folded() {
        use postio_model::{BlobId, DraftId};

        let plan = coalesce(&[
            row(
                1,
                message(1),
                Operation::Send {
                    draft: DraftId::new(1),
                },
            ),
            row(
                2,
                message(1),
                Operation::Send {
                    draft: DraftId::new(1),
                },
            ),
            row(
                3,
                message(1),
                Operation::Append {
                    mailbox: archive(),
                    blob: BlobId::new("abc"),
                    flags: FlagSet::new(),
                },
            ),
        ]);

        assert_eq!(plan.steps.len(), 3, "two sends are two emails");
    }

    #[test]
    fn nothing_folds_across_a_send() {
        use postio_model::DraftId;

        let plan = coalesce(&[
            row(1, message(1), set("\\Seen")),
            row(
                2,
                message(1),
                Operation::Send {
                    draft: DraftId::new(1),
                },
            ),
            row(3, message(1), set("\\Flagged")),
        ]);

        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].operation, set("\\Seen"));
        assert_eq!(plan.steps[2].operation, set("\\Flagged"));
    }

    #[test]
    fn expunges_are_left_alone() {
        let plan = coalesce(&[
            row(
                1,
                OperationTarget::Mailbox(trash()),
                Operation::Expunge { mailbox: trash() },
            ),
            row(
                2,
                OperationTarget::Mailbox(trash()),
                Operation::Expunge { mailbox: trash() },
            ),
        ]);

        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn steps_keep_the_order_the_user_acted_in() {
        let plan = coalesce(&[
            row(1, message(1), set("\\Seen")),
            row(2, message(2), moved(inbox(), archive())),
            row(3, message(1), set("\\Flagged")),
            row(4, message(3), set("\\Seen")),
        ]);

        let heads: Vec<i64> = plan.steps.iter().map(|step| step.head().get()).collect();
        assert_eq!(
            heads,
            vec![1, 2, 4],
            "message 1's two flag changes fold and keep row 1's place"
        );
        assert_eq!(plan.steps[0].rows.len(), 2);
    }

    #[test]
    fn an_empty_batch_plans_nothing() {
        let plan = coalesce(&[]);

        assert!(plan.steps.is_empty());
        assert!(plan.obsolete.is_empty());
        assert_eq!(plan.rows(), 0);
        assert_eq!(plan.saved(), 0);
    }

    #[test]
    fn every_row_in_a_batch_is_accounted_for() {
        let batch = [
            row(1, message(1), set("\\Seen")),
            row(2, message(1), clear("\\Seen")),
            row(3, message(2), moved(inbox(), archive())),
            row(4, message(2), moved(archive(), trash())),
            row(5, message(3), set("\\Flagged")),
        ];
        let plan = coalesce(&batch);

        assert_eq!(
            plan.rows(),
            batch.len(),
            "a row that is neither planned nor obsolete would be a lost mutation"
        );
    }

    #[test]
    fn a_run_of_autosaves_folds_into_one_upload() {
        let drafts = MailboxId::new(4);
        let batch = [
            row(1, draft(7), Operation::SaveDraft { mailbox: drafts }),
            row(2, draft(7), Operation::SaveDraft { mailbox: drafts }),
            row(3, draft(7), Operation::SaveDraft { mailbox: drafts }),
        ];

        let plan = coalesce(&batch);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].rows.len(), 3, "all three settle together");
        assert_eq!(plan.saved(), 2);
    }

    #[test]
    fn discarding_subsumes_a_save_that_has_not_gone_out() {
        let drafts = MailboxId::new(4);
        let discard = Operation::DiscardDraft {
            mailbox: drafts,
            remote_id: postio_model::RemoteId::new("1:9"),
        };
        let batch = [
            row(1, draft(7), Operation::SaveDraft { mailbox: drafts }),
            row(2, draft(7), discard.clone()),
        ];

        let plan = coalesce(&batch);

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.steps[0].operation, discard,
            "uploading a copy only to remove it is two round trips to nowhere"
        );
    }

    #[test]
    fn saves_for_different_drafts_stay_apart() {
        let drafts = MailboxId::new(4);
        let batch = [
            row(1, draft(7), Operation::SaveDraft { mailbox: drafts }),
            row(2, draft(8), Operation::SaveDraft { mailbox: drafts }),
        ];

        assert_eq!(coalesce(&batch).steps.len(), 2);
    }
}
