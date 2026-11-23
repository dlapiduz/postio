//! Focus's reminders (spec 007 US5, research R7): a conversation that returns to the top of Focus's
//! inbox, marked "No reply since", when nobody but the person has written in it by a time. One
//! standing reminder per conversation: setting another replaces it.
//! Cancelled by the filing pass when someone else's message joins the conversation (settled if it
//! had surfaced); fired by the due timer on the engine's tick. Like a snooze, a reminder lives in
//! the store, so a resync forgets it (research R14).

use chrono::{DateTime, Utc};
use postio_model::ids::{MailboxId, MessageId, ReminderId, ThreadId};

use super::{from_millis, to_millis};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: ReminderId,
    pub thread: ThreadId,
    pub anchor: MessageId,
    pub set_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
}

impl Reminder {
    pub fn is_surfaced(&self) -> bool {
        self.fired_at.is_some() && self.cancelled_at.is_none() && self.settled_at.is_none()
    }
}

#[derive(Debug)]
pub struct ReminderRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str =
    "id, thread_id, anchor_message_id, set_at, due_at, fired_at, cancelled_at, settled_at";

const STANDING: &str = "cancelled_at IS NULL AND settled_at IS NULL";

impl<'a> ReminderRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn standing(&self, thread: ThreadId) -> Result<Option<Reminder>> {
        sql::first(
            self.connection,
            &format!(
                "SELECT {COLUMNS} FROM reminders WHERE thread_id = ?1 AND {STANDING}
                  ORDER BY id DESC LIMIT 1"
            ),
            [thread.get()],
            read_reminder,
        )
        .await
    }

    /// One reminder stands per conversation, so setting another replaces the one standing, fired or
    /// not: a surfaced row set again goes back to waiting.
    pub async fn set(
        &self,
        thread: ThreadId,
        anchor: MessageId,
        due_at: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<ReminderId> {
        sql::in_scope(self.connection, |scope| async move {
            let repository = ReminderRepository::new(&scope);
            if let Some(standing) = repository.standing(thread).await? {
                sql::execute(
                    &scope,
                    "UPDATE reminders
                        SET anchor_message_id = ?2, set_at = ?3, due_at = ?4, fired_at = NULL
                      WHERE id = ?1",
                    bind![
                        standing.id.get(),
                        anchor.get(),
                        to_millis(at),
                        to_millis(due_at)
                    ],
                )
                .await?;
                return Ok(standing.id);
            }
            sql::execute(
                &scope,
                "INSERT INTO reminders (thread_id, anchor_message_id, set_at, due_at)
                 VALUES (?1, ?2, ?3, ?4)",
                bind![thread.get(), anchor.get(), to_millis(at), to_millis(due_at)],
            )
            .await?;
            Ok(ReminderId::new(scope.last_insert_rowid()))
        })
        .await
    }

    /// Gone rather than marked: the person's own "never mind", or undo of setting it.
    pub async fn clear(&self, thread: ThreadId) -> Result<bool> {
        let cleared = sql::execute(
            self.connection,
            &format!("DELETE FROM reminders WHERE thread_id = ?1 AND {STANDING}"),
            [thread.get()],
        )
        .await?;
        Ok(cleared > 0)
    }

    /// One statement, a seek per conversation on `idx_reminders_thread`.
    pub async fn standing_on(&self, threads: &[ThreadId]) -> Result<Vec<Reminder>> {
        if threads.is_empty() {
            return Ok(Vec::new());
        }
        sql::all(
            self.connection,
            &Self::explain_standing_on(threads.len()),
            threads
                .iter()
                .map(|thread| thread.get())
                .collect::<Vec<_>>(),
            read_reminder,
        )
        .await
    }

    pub fn explain_standing_on(threads: usize) -> String {
        format!(
            "SELECT {COLUMNS} FROM reminders WHERE thread_id IN ({}) AND {STANDING}",
            super::messages::placeholders(threads, 1)
        )
    }

    /// One statement, over `idx_reminders_standing`.
    pub async fn due(&self, now: DateTime<Utc>) -> Result<Vec<Reminder>> {
        sql::all(
            self.connection,
            Self::explain_due(),
            [to_millis(now)],
            read_reminder,
        )
        .await
    }

    pub fn explain_due() -> &'static str {
        "SELECT id, thread_id, anchor_message_id, set_at, due_at, fired_at, cancelled_at,
                settled_at
           FROM reminders
          WHERE fired_at IS NULL AND settled_at IS NULL AND cancelled_at IS NULL
            AND due_at <= ?1
          ORDER BY due_at LIMIT 256"
    }

    pub async fn fire(&self, id: ReminderId, at: DateTime<Utc>) -> Result<bool> {
        let fired = sql::execute(
            self.connection,
            &format!(
                "UPDATE reminders SET fired_at = ?2
                  WHERE id = ?1 AND fired_at IS NULL AND {STANDING}"
            ),
            bind![id.get(), to_millis(at)],
        )
        .await?;
        Ok(fired > 0)
    }

    pub async fn cancel(&self, id: ReminderId, at: DateTime<Utc>) -> Result<bool> {
        let cancelled = sql::execute(
            self.connection,
            &format!(
                "UPDATE reminders SET cancelled_at = ?2
                  WHERE id = ?1 AND fired_at IS NULL AND {STANDING}"
            ),
            bind![id.get(), to_millis(at)],
        )
        .await?;
        Ok(cancelled > 0)
    }

    pub async fn settle(&self, id: ReminderId, at: DateTime<Utc>) -> Result<bool> {
        let settled = sql::execute(
            self.connection,
            &format!("UPDATE reminders SET settled_at = ?2 WHERE id = ?1 AND {STANDING}"),
            bind![id.get(), to_millis(at)],
        )
        .await?;
        Ok(settled > 0)
    }

    /// Senders of every message received after the reminder was set; whether one is somebody other
    /// than the person is the caller's to say. One statement, a seek on the conversation's own
    /// index.
    pub async fn writers_since(
        &self,
        thread: ThreadId,
        since: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        sql::all(
            self.connection,
            Self::explain_writers_since(),
            bind![thread.get(), to_millis(since)],
            |row| row.col(0),
        )
        .await
    }

    pub fn explain_writers_since() -> &'static str {
        "SELECT DISTINCT a.address_normalized
           FROM messages m
           JOIN recipients r ON r.message_id = m.id AND r.kind = 'from'
           JOIN addresses a ON a.id = r.address_id
          WHERE m.thread_id = ?1 AND m.received_at > ?2
          LIMIT 256"
    }

    /// One statement, over `idx_reminders_standing`.
    pub async fn surfaced(&self) -> Result<Vec<Reminder>> {
        sql::all(self.connection, Self::explain_surfaced(), (), read_reminder).await
    }

    /// A conversation archived away takes its row with it, and an undone archive brings it back,
    /// with nothing written to the reminder. One statement: standing reminders through
    /// `idx_reminders_standing`, each asking for one message in an inbox through its conversation's
    /// index.
    pub async fn surfaced_in(&self, inboxes: &[MailboxId]) -> Result<Vec<Reminder>> {
        if inboxes.is_empty() {
            return Ok(Vec::new());
        }
        sql::all(
            self.connection,
            &Self::explain_surfaced_in(inboxes.len()),
            inboxes.iter().map(|inbox| inbox.get()).collect::<Vec<_>>(),
            read_reminder,
        )
        .await
    }

    pub fn explain_surfaced_in(inboxes: usize) -> String {
        let placeholders = (1..=inboxes)
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT r.id, r.thread_id, r.anchor_message_id, r.set_at, r.due_at, r.fired_at,
                    r.cancelled_at, r.settled_at
               FROM reminders r
              WHERE r.fired_at IS NOT NULL AND r.settled_at IS NULL AND r.cancelled_at IS NULL
                AND EXISTS (SELECT 1 FROM messages m
                             WHERE m.thread_id = r.thread_id
                               AND m.mailbox_id IN ({placeholders})
                               AND m.{visible})
              ORDER BY r.fired_at, r.id LIMIT 256",
            visible = super::VISIBLE
        )
    }

    pub fn explain_surfaced() -> &'static str {
        "SELECT id, thread_id, anchor_message_id, set_at, due_at, fired_at, cancelled_at,
                settled_at
           FROM reminders
          WHERE fired_at IS NOT NULL AND settled_at IS NULL AND cancelled_at IS NULL
          ORDER BY fired_at, id LIMIT 256"
    }
}

fn read_reminder(row: &Row) -> Result<Reminder> {
    let time = |index: usize| -> Result<Option<DateTime<Utc>>> {
        Ok(row.col::<Option<i64>>(index)?.map(from_millis))
    };
    Ok(Reminder {
        id: ReminderId::new(row.col(0)?),
        thread: ThreadId::new(row.col(1)?),
        anchor: MessageId::new(row.col(2)?),
        set_at: from_millis(row.col(3)?),
        due_at: from_millis(row.col(4)?),
        fired_at: time(5)?,
        cancelled_at: time(6)?,
        settled_at: time(7)?,
    })
}
