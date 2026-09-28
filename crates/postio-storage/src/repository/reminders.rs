//! Focus's reminders (spec 007 US5, research R7): a conversation that comes
//! back to the top of Focus's inbox, marked "No reply since", when nobody
//! but the person has written in it by a time.
//!
//! # Who writes, who reads
//!
//! - **Set and cleared** by `remind_if_no_reply` (the session's verb), one
//!   standing reminder per conversation: setting another replaces it.
//! - **Cancelled** by Focus's filing pass, in the transaction that filed a
//!   message from somebody else into the conversation (FR-044), and
//!   **settled** there when the reminder had already surfaced.
//! - **Fired** by Focus's due timer, on the engine's tick, once `due_at` has
//!   passed with no reply: that is when the conversation surfaces.
//!
//! Like a snooze, a reminder lives in the store and not in `config.toml`, so
//! a resync forgets it (research R14).

use chrono::{DateTime, Utc};
use postio_model::ids::{MessageId, ReminderId, ThreadId};

use super::{from_millis, to_millis};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// One reminder, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    /// Its row.
    pub id: ReminderId,
    /// The conversation it is about.
    pub thread: ThreadId,
    /// The message it was set on: the one the person waits on a reply to.
    pub anchor: MessageId,
    /// When it was set: the "since" of "No reply since".
    pub set_at: DateTime<Utc>,
    /// When it falls due.
    pub due_at: DateTime<Utc>,
    /// When it fired, finding no reply: from then on it is surfaced.
    pub fired_at: Option<DateTime<Utc>>,
    /// When a reply cancelled it, before it fired.
    pub cancelled_at: Option<DateTime<Utc>>,
    /// When a surfaced reminder stopped standing: a reply came after all.
    pub settled_at: Option<DateTime<Utc>>,
}

impl Reminder {
    /// Whether it has fired and still stands: a surfaced row of Focus's
    /// inbox.
    pub fn is_surfaced(&self) -> bool {
        self.fired_at.is_some() && self.cancelled_at.is_none() && self.settled_at.is_none()
    }
}

/// Reads and writes [`Reminder`]s.
#[derive(Debug)]
pub struct ReminderRepository<'a> {
    connection: &'a Connection,
}

/// Every column, in the order [`read_reminder`] reads them.
const COLUMNS: &str =
    "id, thread_id, anchor_message_id, set_at, due_at, fired_at, cancelled_at, settled_at";

/// What "still stands" means: nothing has cancelled it or settled it.
const STANDING: &str = "cancelled_at IS NULL AND settled_at IS NULL";

impl<'a> ReminderRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The reminder standing on `thread`, fired or not: at most one.
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

    /// Sets `thread`'s reminder, waiting on a reply to `anchor`, due at
    /// `due_at`, as of `at`, and answers its row.
    ///
    /// One reminder stands per conversation, so setting another replaces the
    /// one standing, fired or not: the person has named a new time, and a
    /// surfaced row they set again goes back to waiting for it.
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

    /// Takes away the reminder standing on `thread`, and answers whether
    /// there was one. Gone rather than marked: it is the person's own
    /// "never mind", or undo taking back the setting of it.
    pub async fn clear(&self, thread: ThreadId) -> Result<bool> {
        let cleared = sql::execute(
            self.connection,
            &format!("DELETE FROM reminders WHERE thread_id = ?1 AND {STANDING}"),
            [thread.get()],
        )
        .await?;
        Ok(cleared > 0)
    }

    /// The reminders standing on any of `threads`: what the filing pass asks
    /// of the conversations its arrivals joined. One statement, a seek per
    /// conversation on `idx_reminders_thread`.
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

    /// The SQL [`Self::standing_on`] runs for `threads` conversations.
    pub fn explain_standing_on(threads: usize) -> String {
        format!(
            "SELECT {COLUMNS} FROM reminders WHERE thread_id IN ({}) AND {STANDING}",
            super::messages::placeholders(threads, 1)
        )
    }

    /// The reminders that have come due by `now` and not fired: what the due
    /// timer asks each tick. One statement, over `idx_reminders_standing`.
    pub async fn due(&self, now: DateTime<Utc>) -> Result<Vec<Reminder>> {
        sql::all(
            self.connection,
            Self::explain_due(),
            [to_millis(now)],
            read_reminder,
        )
        .await
    }

    /// The SQL [`Self::due`] runs.
    pub fn explain_due() -> &'static str {
        "SELECT id, thread_id, anchor_message_id, set_at, due_at, fired_at, cancelled_at,
                settled_at
           FROM reminders
          WHERE fired_at IS NULL AND settled_at IS NULL AND cancelled_at IS NULL
            AND due_at <= ?1
          ORDER BY due_at LIMIT 256"
    }

    /// Fires `id` at `at`, and answers whether it was still waiting to.
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

    /// Cancels `id` at `at`, because somebody replied before it fired, and
    /// answers whether it was still waiting.
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

    /// Settles `id` at `at`, because a reply came after it surfaced, and
    /// answers whether it was still standing.
    pub async fn settle(&self, id: ReminderId, at: DateTime<Utc>) -> Result<bool> {
        let settled = sql::execute(
            self.connection,
            &format!("UPDATE reminders SET settled_at = ?2 WHERE id = ?1 AND {STANDING}"),
            bind![id.get(), to_millis(at)],
        )
        .await?;
        Ok(settled > 0)
    }

    /// Who has written in `thread` since `since`, normalised: the senders of
    /// every message received after the reminder was set. Whether one of
    /// them is somebody other than the person is the caller's to say, from
    /// the person's own addresses. One statement, a seek on the
    /// conversation's own index.
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

    /// The SQL [`Self::writers_since`] runs.
    pub fn explain_writers_since() -> &'static str {
        "SELECT DISTINCT a.address_normalized
           FROM messages m
           JOIN recipients r ON r.message_id = m.id AND r.kind = 'from'
           JOIN addresses a ON a.id = r.address_id
          WHERE m.thread_id = ?1 AND m.received_at > ?2
          LIMIT 256"
    }

    /// Every reminder that has fired and still stands, oldest first: the
    /// surfaced rows of Focus's inbox. One statement, over
    /// `idx_reminders_standing`.
    pub async fn surfaced(&self) -> Result<Vec<Reminder>> {
        sql::all(self.connection, Self::explain_surfaced(), (), read_reminder).await
    }

    /// The SQL [`Self::surfaced`] runs.
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
