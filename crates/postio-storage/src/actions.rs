//! The mutating half of an action: the rows, and the operation that follows.
//!
//! # Why these live here and not in `postio-session`
//!
//! ADR 0008 Q5 decides that a rule's action is not its own kind of write:
//!
//! > Every action is local-first, exactly like a keystroke: SQLite write,
//! > enqueue the remote operation, emit the event. **There is no rules-only
//! > mutation path**, which means rules inherit offline behaviour,
//! > reconciliation and event flow for free.
//!
//! Two callers therefore need the same verb — the command bus, when a person
//! presses `a`, and the rules pass, when a message arrives — and ADR 0028
//! puts the shared half below both rather than beside either. The half that
//! moves down is the one that touches SQLite; what stays up in
//! [`postio_session::actions`] is everything that only a person's gesture
//! has: resolving what the selection meant, pushing an undo entry, and
//! emitting the events the panes repaint from.
//!
//! Nothing here knows what a `Command`, an `UndoKind` or an `Applied` is, and
//! `check-crate-boundaries.py` is what keeps it that way.
//!
//! # Why the caller owns the transaction
//!
//! ADR 0008 Q3:
//!
//! > Header-only rules run in the sync pass that inserts the message, **in
//! > the same transaction as the insert, before any event is emitted.** The
//! > user never sees it land in the Inbox first.
//!
//! The sync pass owns that transaction, so a verb here cannot open one of its
//! own — it would commit the filing separately from the insert that brought
//! the message in, and a failure between the two would leave a message filed
//! by a rule and never inserted. The interactive path opens a transaction and
//! passes it in; the rules pass hands over the one it is already inside.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::Transaction;

use postio_model::{
    AccountId, Flag, FlagSet, MailboxId, Message, MessageId, Operation, OperationTarget, ThreadId,
};

use crate::Result;
use crate::repository::{
    FlagSource, MessageRepository, OperationQueueRepository, ThreadOrder, ThreadRepository,
};

/// Which server operation a relocation is.
///
/// The only thing the caller chooses about the queue row, and deliberately
/// not `UndoKind`: what a relocation means to the *undo stack* is a session
/// concept, and what it means to the *server* is this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocation {
    /// An ordinary move between two of the account's mailboxes.
    Move,
    /// A move to the account's trash, which the server is told about as a
    /// delete. Never a hard delete — the mail is recoverable exactly as a
    /// hand-trashed message is.
    Trash,
}

/// Move messages to `destination` and enqueue the operation that tells the
/// server.
///
/// `by_source` groups the messages by the mailbox they are leaving, because
/// the operation names that mailbox: one `enqueue_many` per source rather
/// than one `enqueue` per message, so a multi-select spanning a handful of
/// folders costs a handful of statements rather than one per row.
///
/// Every message must belong to `account` and every destination must be one
/// of that account's own mailboxes. A destination in another account is not a
/// move at all but ADR 0005 Q9's three-phase saga, and resolving that is the
/// caller's business — this verb would write a row claiming a mailbox the
/// account does not own.
pub fn relocate(
    transaction: &Transaction<'_>,
    account: AccountId,
    by_source: &BTreeMap<MailboxId, Vec<MessageId>>,
    destination: MailboxId,
    relocation: Relocation,
    at: DateTime<Utc>,
) -> Result<()> {
    let messages = MessageRepository::new(transaction);
    let queue = OperationQueueRepository::new(transaction);
    for (source, ids) in by_source {
        let operation = match relocation {
            Relocation::Trash => Operation::Delete {
                from: *source,
                trash: destination,
            },
            Relocation::Move => Operation::Move {
                from: *source,
                to: destination,
            },
        };
        // The queue row first, as `postio_session::actions` has always done
        // it. The comment there says the move would otherwise null the
        // coordinates the enqueue snapshots (#289); that is not quite what
        // the code does today -- `enqueue_many` snapshots `remote_id`, and
        // `move_to` nulls `uid`, `uid_validity` and `mod_seq` but not
        // `remote_id` -- so the two orders currently produce identical rows,
        // and no test here can tell them apart. The order is kept because
        // this is a move and because it is the safe one if the snapshot ever
        // widens to a column the move does clear. See #1125 for the check.
        queue.enqueue_many(account, ids, &operation, at)?;
        messages.move_to(ids, destination)?;
    }
    Ok(())
}

/// Set or clear one flag on `rows`, and keep their threads honest.
///
/// Answers the messages whose rows a caller should repaint: every message in
/// every thread the change touched, not only the ones whose flags moved. The
/// threads keep two things the rows alone cannot (#754) -- the denormalised
/// aggregates the account-scoped page reads straight off `threads`, and the
/// membership that names the row the conversation list draws -- so both are
/// recomputed inside the caller's transaction. Nothing can then observe a
/// read message under an unread thread.
///
/// `rows` must already have been read: the verb needs each message's current
/// flags to compute the new set, and its thread to recompute. Rows whose flag
/// is already `wanted` are the caller's to filter -- this writes what it is
/// given.
pub fn set_flag(
    transaction: &Transaction<'_>,
    account: AccountId,
    rows: &[&Message],
    flag: &Flag,
    wanted: bool,
    at: DateTime<Utc>,
) -> Result<Vec<MessageId>> {
    let one: FlagSet = std::iter::once(flag.clone()).collect();
    let messages = MessageRepository::new(transaction);
    let queue = OperationQueueRepository::new(transaction);
    for message in rows {
        let mut flags = message.flags.clone();
        if wanted {
            flags.insert(flag.clone());
        } else {
            flags.remove(flag);
        }
        messages.set_flags(message.id, &flags, FlagSource::Local)?;
        let operation = if wanted {
            Operation::SetFlags { flags: one.clone() }
        } else {
            Operation::ClearFlags { flags: one.clone() }
        };
        queue.enqueue(
            account,
            OperationTarget::Message(message.id),
            &operation,
            at,
        )?;
    }

    let threads = ThreadRepository::new(transaction);
    let mut conversations: Vec<ThreadId> = rows.iter().filter_map(|row| row.thread_id).collect();
    conversations.sort_unstable();
    conversations.dedup();
    let mut siblings: Vec<MessageId> = Vec::new();
    for thread in &conversations {
        threads.recompute(*thread)?;
        for row in threads.messages(*thread, ThreadOrder::Oldest)? {
            siblings.push(row.id);
        }
    }
    Ok(siblings)
}
