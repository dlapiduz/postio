//! Sorts what another app filed while Focus was closed, through the same filing pass the engines
//! run, newest first, in the background.
//! A settings mark (the newest message id when Focus last ran) says where to start; with no mark,
//! on first open, nothing is caught up.
//! The row under a client's cursor is recorded as looked at and left.

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

pub(crate) const FILED_THROUGH: &str = "focus.filed_through";

const BATCH: u32 = 50;

const BREATHER: Duration = Duration::from_millis(20);

type Failure = postio_storage::Error;

/// With no mark (Focus's first open) it only sets one.
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

/// A first sync's backlog is never filtered, so it must not be sorted at the next open as if
/// another app had filed it.
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
            super::yield_to_reads(database).await;
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
            let permit = connection
                .write_gate()
                .acquire(WritePriority::Background)
                .await;
            let batch = &batch;
            let effects = postio_storage::transaction(&connection, |transaction| async move {
                // A failure takes back only what the pass wrote; the batch is recorded all the same
                // so it is not retried.
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
            // Given back before the breather: held through it, no other task had run to take it
            // and the body stage waited out the whole catch-up for its turn.
            drop(permit);
            drop(connection);
            announce(inner, account, inbox, &effects);
            taken += batch.len();
            tokio::time::sleep(BREATHER).await;
        }
    }
    Ok(taken)
}

fn under_a_cursor(inner: &Inner) -> Vec<MessageId> {
    inner
        .clients
        .lock()
        .expect("never poisoned")
        .values()
        .filter_map(|entry| entry.state.snapshot().focus())
        .collect()
}

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
