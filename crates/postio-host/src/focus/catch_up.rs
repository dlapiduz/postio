//! Sorting what another app filed while Focus was closed (spec 007 T127,
//! FR-134, US9 scenario 8).
//!
//! Focus's rules act only while Focus runs: mail another app files lands in
//! the inbox as it always has. When Focus next opens, it files that mail as
//! if it had filed it itself -- filtered with its reason, or held for its
//! digest -- in the background, newest first, through the same filing pass
//! the engines run. So a notification the user saw in the classic inbox can
//! then leave it, the way an archive leaves.
//!
//! # Since when
//!
//! A mark in the store's settings says how far Focus had accounted for the
//! mail: the newest message id when it last ran. Ids only grow, so what
//! another app filed since is exactly what is past the mark. Focus keeps the
//! mark current while it runs ([`keep_mark`], on the due timer's tick, and
//! [`mark_newest`] as it stops), which also covers a new account's first
//! sync: a first sync files the backlog, never new mail, and is never
//! filtered (FR-118).
//!
//! On Focus's first open there is no mark, and nothing is caught up on:
//! filtering applies to mail filed after it is turned on, and the first
//! open is what turns it on (FR-118, FR-119). Moving what is already in the
//! inbox is the deliberate sweep's (T128).
//!
//! # What it never moves
//!
//! The row under a client's cursor stays where it is (spec, Edge Cases): it
//! is recorded as looked at, and left.

use std::sync::Arc;
use std::time::Duration;

use postio_core::Event;
use postio_model::{MailboxRole, MessageId};
use postio_storage::repository::{
    FocusClassifiedRepository, FocusStage, MessageRepository, SettingsRepository, ThreadRepository,
};
use postio_storage::{Store, WritePriority};
use postio_sync::{FiledMessage, FilingEffects, FilingPass};

use crate::Inner;

/// The settings key the mark is kept under: the newest message id Focus has
/// accounted for.
pub(crate) const FILED_THROUGH: &str = "focus.filed_through";

/// How many messages one batch takes.
const BATCH: u32 = 50;

/// The pause between two batches: background work, behind a person's own.
const BREATHER: Duration = Duration::from_millis(20);

/// Why the catch-up could not read or write what it meant to.
type Failure = postio_storage::Error;

/// Catch up on what was filed since Focus last ran, then move the mark to
/// where the store is now. Answers how many messages it looked at.
///
/// With no mark -- Focus's first open -- it only sets one.
pub(crate) async fn catch_up(inner: &Arc<Inner>) -> Result<usize, Failure> {
    let database = &inner.wiring.database;
    let connection = database.connect_background().await?;
    let newest = MessageRepository::new(&connection).newest_id().await?;
    let mark = SettingsRepository::new(&connection)
        .get(FILED_THROUGH)
        .await?
        .and_then(|mark| mark.parse::<i64>().ok())
        .map(MessageId::new);
    drop(connection);
    let taken = match (mark, inner.wiring.filing.get()) {
        (Some(mark), Some(pass)) => sort_since(inner, database, mark, pass.as_ref()).await?,
        _ => 0,
    };
    if let Some(newest) = newest {
        set_mark(database, newest).await?;
    }
    if taken > 0 {
        tracing::info!(taken, "Focus sorted mail filed while it was closed");
    }
    Ok(taken)
}

/// Move the mark to the newest message, when it has moved: what Focus's
/// tick does, so the mark says where the store was when Focus last ran.
pub(crate) async fn keep_mark(
    database: &Store,
    marked: &mut Option<MessageId>,
) -> Result<(), Failure> {
    let newest = {
        let reader = database.read().await?;
        MessageRepository::new(&reader).newest_id().await?
    };
    if let Some(newest) = newest
        && *marked != Some(newest)
    {
        set_mark(database, newest).await?;
        *marked = Some(newest);
    }
    Ok(())
}

/// Move the mark to the newest message now: what stopping Focus does, so
/// mail that landed since the last tick -- a first sync's backlog above
/// all, which is never filtered (FR-118) -- is not sorted at the next open
/// as if another app had filed it.
pub(crate) async fn mark_newest(database: &Store) -> Result<(), Failure> {
    keep_mark(database, &mut None).await
}

async fn set_mark(database: &Store, newest: MessageId) -> Result<(), Failure> {
    let connection = database.connect_background().await?;
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    SettingsRepository::new(&connection)
        .set(FILED_THROUGH, &newest.get().to_string())
        .await
}

/// File every inbox's mail past `mark` with `pass`, newest first, a batch
/// at a time.
async fn sort_since(
    inner: &Arc<Inner>,
    database: &Store,
    mark: MessageId,
    pass: &dyn FilingPass,
) -> Result<usize, Failure> {
    let version = postio_classify::VERSION;
    let inboxes = {
        let reader = database.read().await?;
        ThreadRepository::new(&reader).unified_inboxes().await?
    };
    let mut taken = 0;
    for (account, inbox) in inboxes {
        loop {
            let connection = database.connect_background().await?;
            let batch = FocusClassifiedRepository::new(&connection)
                .pending_filings(inbox, mark, version, BATCH)
                .await?;
            if batch.is_empty() {
                break;
            }
            let watched = under_a_cursor(inner);
            let messages = MessageRepository::new(&connection);
            let mut rows = Vec::with_capacity(batch.len());
            for id in &batch {
                if watched.contains(id) {
                    continue;
                }
                if let Some(row) = messages.get(*id).await? {
                    rows.push(row);
                }
            }
            let filed: Vec<FiledMessage<'_>> = rows
                .iter()
                .map(|row| FiledMessage {
                    message: row,
                    thread: row.thread_id,
                    role: MailboxRole::Inbox,
                })
                .collect();
            let _permit = connection
                .write_gate()
                .acquire(WritePriority::Background)
                .await;
            let batch = &batch;
            let effects = postio_storage::transaction(&connection, |transaction| async move {
                // The pass's own scope: a failure takes back only what it
                // wrote, and the batch is recorded all the same, so it is not
                // taken again and again.
                let effects = postio_storage::transaction(&transaction, |scope| async move {
                    pass.file(&scope, &filed).await
                })
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        arrivals = batch.len(),
                        %error,
                        "Focus could not sort mail filed while it was closed: {error}"
                    );
                    FilingEffects::default()
                });
                FocusClassifiedRepository::new(&transaction)
                    .record(batch, FocusStage::Filing, version)
                    .await?;
                Ok::<_, Failure>(effects)
            })
            .await?;
            drop(connection);
            announce(inner, account, inbox, &effects);
            taken += batch.len();
            tokio::time::sleep(BREATHER).await;
        }
    }
    Ok(taken)
}

/// The rows under every client's cursor right now.
fn under_a_cursor(inner: &Inner) -> Vec<MessageId> {
    inner
        .clients
        .lock()
        .expect("never poisoned")
        .values()
        .filter_map(|entry| entry.state.snapshot().focus())
        .collect()
}

/// Tell every client what left the inbox: filtered mail leaves it the way
/// an archive does, and held mail leaves Focus's view of it.
fn announce(
    inner: &Inner,
    account: postio_model::AccountId,
    inbox: postio_model::MailboxId,
    effects: &FilingEffects,
) {
    if !effects.filtered.is_empty() {
        inner.hub.emit(Event::MessagesRemoved {
            account,
            mailbox: inbox,
            messages: effects.filtered.clone(),
        });
    }
    if !effects.held.is_empty() {
        inner.hub.emit(Event::MessageListChanged {
            account,
            mailbox: inbox,
        });
    }
}
