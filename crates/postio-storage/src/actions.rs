//! The mutating half of an action: the rows plus the queued operation, shared by the command bus
//! and the rules pass (ADR 0008, ADR 0028).
//! The caller owns the transaction, so a rule's filing commits with the insert that brought the
//! message in.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use postio_model::{
    AccountId, Flag, FlagSet, MailboxId, Message, MessageId, Operation, OperationTarget, ThreadId,
};

use crate::Result;
use crate::repository::{
    FilterReason, FlagSource, MessageRepository, OperationQueueRepository, ThreadOrder,
    ThreadRepository,
};
use crate::store::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocation {
    Move,
    /// Told to the server as a delete, never a hard delete.
    Trash,
}

/// `by_source` groups messages by the mailbox they leave: one `enqueue_many` per source.
///
/// Every message and destination must belong to `account`; a cross-account move is a saga (ADR 0005
/// Q9), not this.
pub async fn relocate(
    transaction: &Connection,
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
        // Queue first: `enqueue_many` snapshots `remote_id`, which `move_to` clears.
        queue.enqueue_many(account, ids, &operation, at).await?;
        messages.move_to(ids, destination).await?;
    }
    Ok(())
}

/// Call before [`relocate`]: queue rows snapshot the server coordinate the move clears. `rows` must
/// belong to `account`.
pub async fn mark_filtered(
    transaction: &Connection,
    account: AccountId,
    rows: &[&Message],
    reason: Option<FilterReason>,
    at: DateTime<Utc>,
) -> Result<()> {
    let wanted: Option<Flag> = reason.map(FilterReason::keyword);
    let others: FlagSet = FilterReason::ALL
        .into_iter()
        .map(FilterReason::keyword)
        .filter(|keyword| Some(keyword) != wanted.as_ref())
        .collect();
    let messages = MessageRepository::new(transaction);
    let mut cleared = Vec::new();
    let mut added = Vec::new();
    for row in rows {
        let mut flags = row.flags.clone();
        let mut changed = false;
        for keyword in others.iter() {
            changed |= flags.remove(keyword);
        }
        if changed {
            cleared.push(row.id);
        }
        if let Some(keyword) = &wanted
            && flags.insert(keyword.clone())
        {
            added.push(row.id);
            changed = true;
        }
        if changed {
            messages
                .set_flags(row.id, &flags, FlagSource::Local)
                .await?;
        }
    }
    let queue = OperationQueueRepository::new(transaction);
    queue
        .enqueue_many(
            account,
            &cleared,
            &Operation::ClearFlags { flags: others },
            at,
        )
        .await?;
    if let Some(keyword) = wanted {
        let flags = std::iter::once(keyword).collect();
        queue
            .enqueue_many(account, &added, &Operation::SetFlags { flags }, at)
            .await?;
    }
    Ok(())
}

/// Returns the messages to repaint: every message in every touched thread, since thread aggregates
/// and membership are recomputed in the caller's transaction (#754).
///
/// `rows` must already be read; rows already at `wanted` are the caller's to filter.
pub async fn set_flag(
    transaction: &Connection,
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
        messages
            .set_flags(message.id, &flags, FlagSource::Local)
            .await?;
        let operation = if wanted {
            Operation::SetFlags { flags: one.clone() }
        } else {
            Operation::ClearFlags { flags: one.clone() }
        };
        queue
            .enqueue(
                account,
                OperationTarget::Message(message.id),
                &operation,
                at,
            )
            .await?;
    }

    let threads = ThreadRepository::new(transaction);
    let mut conversations: Vec<ThreadId> = rows.iter().filter_map(|row| row.thread_id).collect();
    conversations.sort_unstable();
    conversations.dedup();
    let mut siblings: Vec<MessageId> = Vec::new();
    for thread in &conversations {
        threads.recompute(*thread).await?;
        for row in threads.messages(*thread, ThreadOrder::Oldest).await? {
            siblings.push(row.id);
        }
    }
    Ok(siblings)
}

/// Not undoable (auto-empty of Junk and Trash, #1932). Queue rows first, as in [`relocate`].
pub async fn purge(
    transaction: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    ids: &[MessageId],
    at: DateTime<Utc>,
) -> Result<()> {
    OperationQueueRepository::new(transaction)
        .enqueue_many(account, ids, &Operation::Purge { mailbox }, at)
        .await?;
    MessageRepository::new(transaction)
        .set_deleted_locally(ids, true)
        .await?;
    Ok(())
}
