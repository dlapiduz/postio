//! Threads: the conversation a message belongs to, and the row the list shows.
//!
//! # What is stored and what is derived
//!
//! The `threads` table holds only aggregates — count, unread count, whether
//! anything in the conversation is flagged or has an attachment, when it
//! started and when it last moved. Membership is *not* stored twice: a message
//! belongs to a thread because `messages.thread_id` says so, and everything
//! else ([`Thread::participants`], [`Thread::mailbox_ids`],
//! [`Thread::labels`]) is derived from the members. One place to be wrong is
//! better than two places to keep in step.
//!
//! The aggregates exist because the list reads them on every row and deriving
//! them per row would be a query per row. They are recomputed by
//! [`ThreadRepository::recompute`], which every mutation here calls for you.
//!
//! # No N+1
//!
//! A page of threads needs each row's participants and its newest message.
//! [`ThreadRepository::page`] fetches all three with three statements — the
//! page, then the participants for the whole page, then the newest message for
//! the whole page — never one per row. `tests/threads.rs` counts the statements
//! and fails if that changes.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, EmailAddress, LabelId, MailboxId, MessageId, Thread, ThreadId, normalize_subject,
};
use rusqlite::{Connection, Row, params, params_from_iter};

use super::messages::{LIST_COLUMNS, MessageListRow, placeholders, read_list_row};
use super::{from_millis, require_persisted, to_millis};
use crate::error::{Error, Result};

/// How many threads a page holds when the caller does not say.
pub const DEFAULT_THREAD_PAGE_SIZE: u32 = 50;

/// Which way a thread's messages are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadOrder {
    /// Oldest first: the order the conversation happened in, which is how the
    /// drill-in reads down the page.
    Oldest,
    /// Newest first.
    Newest,
}

/// A position in the thread list: the sort key of the last row already shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadCursor {
    /// The row's `last_at`.
    pub last_at: DateTime<Utc>,
    /// The row's id, which makes the order total.
    pub id: ThreadId,
}

/// One window of the thread list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListQuery {
    /// Whose conversations. Threads never span accounts.
    pub account_id: AccountId,
    /// How many rows at most.
    pub limit: u32,
    /// Where to resume; `None` starts at the most recently active thread.
    pub after: Option<ThreadCursor>,
}

impl ThreadListQuery {
    /// Every thread in an account.
    pub fn account(account_id: AccountId) -> Self {
        Self {
            account_id,
            limit: DEFAULT_THREAD_PAGE_SIZE,
            after: None,
        }
    }

    /// Sets the window size.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Resumes after `cursor`.
    pub fn after(mut self, cursor: ThreadCursor) -> Self {
        self.after = Some(cursor);
        self
    }
}

/// One row of the threaded message list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListRow {
    /// Local id.
    pub id: ThreadId,
    /// Normalized subject of the conversation's root message.
    pub subject: Option<String>,
    /// Everyone who has written in the thread, in first-seen order.
    pub participants: Vec<EmailAddress>,
    /// How many messages the list would show.
    pub message_count: u32,
    /// How many of them are unread.
    pub unread_count: u32,
    /// Whether any member carries an attachment.
    pub has_attachments: bool,
    /// Whether any member is flagged.
    pub is_flagged: bool,
    /// When the conversation started.
    pub first_at: DateTime<Utc>,
    /// When it last moved; the sort key.
    pub last_at: DateTime<Utc>,
    /// The newest message, which is what the row's snippet and sender come
    /// from. `None` only for a thread whose messages have all been hidden.
    pub latest: Option<MessageListRow>,
}

impl ThreadListRow {
    /// The cursor that resumes paging immediately after this row.
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            last_at: self.last_at,
            id: self.id,
        }
    }

    /// Whether anything in the conversation is unread.
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }
}

/// Reads and writes [`Thread`] rows.
#[derive(Debug)]
pub struct ThreadRepository<'a> {
    connection: &'a Connection,
}

const THREAD_COLUMNS: &str = "\
id, account_id, subject, message_count, unread_count, has_attachments, is_flagged,
first_at, last_at";

/// A member of a thread, for the purposes of every aggregate here.
///
/// A message hidden pending a remote delete is not one: the list does not show
/// it, so it must not be in the count, in the drill-in or in the participants.
const MEMBER: &str = "deleted_locally = 0";

impl<'a> ThreadRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a thread, assigning its id.
    ///
    /// The aggregates are whatever the value carries; they become true once
    /// messages are added, because every mutation here recomputes them.
    pub fn create(&self, thread: &mut Thread) -> Result<ThreadId> {
        let account_id = require_persisted(thread.account_id.get(), "account")?;
        self.connection.execute(
            "INSERT INTO threads (account_id, subject, message_count, unread_count,
                                  has_attachments, is_flagged, first_at, last_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                thread.subject,
                thread.message_count,
                thread.unread_count,
                thread.has_attachments,
                thread.is_flagged,
                to_millis(thread.first_at),
                to_millis(thread.last_at),
            ],
        )?;
        thread.id = ThreadId::new(self.connection.last_insert_rowid());
        Ok(thread.id)
    }

    /// Writes a thread's aggregates back.
    ///
    /// Membership is not part of this: a message joins a thread through
    /// [`ThreadRepository::add_message`], never by being listed here.
    pub fn update(&self, thread: &Thread) -> Result<()> {
        let id = require_persisted(thread.id.get(), "thread")?;
        let changed = self.connection.execute(
            "UPDATE threads
                SET account_id = ?2, subject = ?3, message_count = ?4, unread_count = ?5,
                    has_attachments = ?6, is_flagged = ?7, first_at = ?8, last_at = ?9
              WHERE id = ?1",
            params![
                id,
                thread.account_id.get(),
                thread.subject,
                thread.message_count,
                thread.unread_count,
                thread.has_attachments,
                thread.is_flagged,
                to_millis(thread.first_at),
                to_millis(thread.last_at),
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "thread",
                id,
            });
        }
        Ok(())
    }

    /// One thread, with its membership and participants derived.
    pub fn get(&self, id: ThreadId) -> Result<Option<Thread>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mut thread = read_thread(row)?;
        drop(rows);
        drop(statement);

        thread.message_ids = self.member_ids(id)?;
        thread.participants = self
            .participants_for(&[id])?
            .remove(&id)
            .unwrap_or_default();
        thread.mailbox_ids = self.mailboxes_in(id)?;
        thread.labels = self.labels_in(id)?;
        Ok(Some(thread))
    }

    /// Deletes a thread, returning whether there was one.
    ///
    /// Its messages survive with no thread: threading is a local derivation and
    /// can simply run again.
    pub fn delete(&self, id: ThreadId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM threads WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    /// Puts a message in a thread and brings the aggregates up to date.
    ///
    /// If the message was in another thread, that one is recomputed too — an
    /// abandoned thread that still claims the message would show a count the
    /// drill-in cannot produce.
    pub fn add_message(&self, thread_id: ThreadId, message_id: MessageId) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let previous = thread_of(&transaction, message_id)?;

        let changed = transaction.execute(
            "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            params![message_id.get(), thread_id.get()],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: message_id.get(),
            });
        }

        recompute_in(&transaction, thread_id)?;
        if let Some(previous) = previous.filter(|previous| *previous != thread_id) {
            recompute_in(&transaction, previous)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Takes a message out of whatever thread it is in.
    pub fn remove_message(&self, message_id: MessageId) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let previous = thread_of(&transaction, message_id)?;
        transaction.execute(
            "UPDATE messages SET thread_id = NULL WHERE id = ?1",
            [message_id.get()],
        )?;
        if let Some(previous) = previous {
            recompute_in(&transaction, previous)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Recomputes a thread's aggregates from its members.
    pub fn recompute(&self, id: ThreadId) -> Result<()> {
        recompute_in(self.connection, id)
    }

    /// Moves every message from `absorb` into `keep` and deletes `absorb`.
    ///
    /// This is what a late-arriving parent does: two conversations turn out to
    /// have been one all along. Merging into the older thread keeps the id the
    /// UI may already be showing.
    pub fn merge(&self, keep: ThreadId, absorb: ThreadId) -> Result<()> {
        if keep == absorb {
            return self.recompute(keep);
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2",
            params![keep.get(), absorb.get()],
        )?;
        // Drafts are shown inline in their thread, so they have to follow.
        transaction.execute(
            "UPDATE drafts SET thread_id = ?1 WHERE thread_id = ?2",
            params![keep.get(), absorb.get()],
        )?;
        transaction.execute("DELETE FROM threads WHERE id = ?1", [absorb.get()])?;
        recompute_in(&transaction, keep)?;
        transaction.commit()?;
        Ok(())
    }

    /// A thread's messages as list rows, in either direction.
    pub fn messages(&self, id: ThreadId, order: ThreadOrder) -> Result<Vec<MessageListRow>> {
        let mut statement = self.connection.prepare(&self.explain_messages(order))?;
        let rows = statement.query_map([id.get()], read_list_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The SQL [`ThreadRepository::messages`] runs, for `EXPLAIN QUERY PLAN`.
    pub fn explain_messages(&self, order: ThreadOrder) -> String {
        let direction = match order {
            ThreadOrder::Oldest => "ASC",
            ThreadOrder::Newest => "DESC",
        };
        format!(
            "SELECT {LIST_COLUMNS} FROM messages
              WHERE messages.thread_id = ?1 AND messages.{MEMBER}
              ORDER BY messages.received_at {direction}, messages.id {direction}"
        )
    }

    /// One window of the thread list, most recently active first.
    pub fn page(&self, query: &ThreadListQuery) -> Result<Vec<ThreadListRow>> {
        let mut statement = self.connection.prepare(&self.explain(query))?;
        let mut arguments = vec![query.account_id.get()];
        if let Some(cursor) = query.after {
            arguments.push(to_millis(cursor.last_at));
            arguments.push(cursor.id.get());
        }
        let rows = statement.query_map(params_from_iter(arguments), |row| {
            Ok(ThreadListRow {
                id: ThreadId::new(row.get(0)?),
                subject: row.get(2)?,
                participants: Vec::new(),
                message_count: row.get(3)?,
                unread_count: row.get(4)?,
                has_attachments: row.get(5)?,
                is_flagged: row.get(6)?,
                first_at: from_millis(row.get(7)?),
                last_at: from_millis(row.get(8)?),
                latest: None,
            })
        })?;
        let mut page: Vec<ThreadListRow> = rows.collect::<Result<_, _>>()?;
        drop(statement);

        // Two more statements for the whole page, rather than two per row.
        let ids: Vec<ThreadId> = page.iter().map(|row| row.id).collect();
        let mut participants = self.participants_for(&ids)?;
        let mut latest = self.latest_messages_for(&ids)?;
        for row in &mut page {
            row.participants = participants.remove(&row.id).unwrap_or_default();
            row.latest = latest.remove(&row.id);
        }
        Ok(page)
    }

    /// The SQL a thread page runs, for `EXPLAIN QUERY PLAN`.
    pub fn explain(&self, query: &ThreadListQuery) -> String {
        // `message_count > 0` hides a conversation whose messages have all been
        // hidden: an empty row is not something the user can act on.
        let cursor = if query.after.is_some() {
            " AND (last_at, id) < (?2, ?3)"
        } else {
            ""
        };
        format!(
            "SELECT {THREAD_COLUMNS} FROM threads
              WHERE account_id = ?1 AND message_count > 0{cursor}
              ORDER BY last_at DESC, id DESC LIMIT {}",
            query.limit
        )
    }

    /// How many threads the list would show.
    pub fn count(&self, account_id: AccountId) -> Result<u32> {
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM threads WHERE account_id = ?1 AND message_count > 0",
            [account_id.get()],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// The members of a thread, oldest first.
    fn member_ids(&self, id: ThreadId) -> Result<Vec<MessageId>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id FROM messages WHERE thread_id = ?1 AND {MEMBER}
              ORDER BY received_at, id"
        ))?;
        let rows = statement.query_map([id.get()], |row| Ok(MessageId::new(row.get(0)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Everyone who has written in each of `ids`, in first-seen order.
    ///
    /// One statement for the whole page. `min(received_at)` is what orders
    /// them, and SQLite takes the bare `name`/`address` columns from the row
    /// that minimum came from — so the display name is the one the participant
    /// first appeared under.
    fn participants_for(&self, ids: &[ThreadId]) -> Result<HashMap<ThreadId, Vec<EmailAddress>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "SELECT messages.thread_id, recipients.name, recipients.address,
                    min(messages.received_at) AS first_seen
               FROM messages
               JOIN recipients ON recipients.message_id = messages.id
              WHERE messages.thread_id IN ({}) AND messages.{MEMBER}
                AND recipients.kind = 'from'
              GROUP BY messages.thread_id, recipients.address_normalized
              ORDER BY messages.thread_id, first_seen, recipients.id",
            placeholders(ids.len(), 1)
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(ids.iter().map(|id| id.get())), |row| {
            Ok((
                ThreadId::new(row.get(0)?),
                EmailAddress::new(row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?),
            ))
        })?;

        let mut participants: HashMap<ThreadId, Vec<EmailAddress>> = HashMap::new();
        for row in rows {
            let (thread_id, address) = row?;
            participants.entry(thread_id).or_default().push(address);
        }
        Ok(participants)
    }

    /// The newest message in each of `ids`, as a list row.
    ///
    /// One statement for the whole page: `row_number()` picks the newest per
    /// thread, and the sender lookups then run only for the rows that survive.
    fn latest_messages_for(&self, ids: &[ThreadId]) -> Result<HashMap<ThreadId, MessageListRow>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "WITH ranked AS (
                 SELECT id, thread_id, subject, preview, received_at, seen, flagged, answered,
                        draft, has_attachments, size,
                        row_number() OVER (PARTITION BY thread_id
                                           ORDER BY received_at DESC, id DESC) AS rank
                   FROM messages
                  WHERE thread_id IN ({}) AND {MEMBER}
             )
             SELECT messages.id, messages.thread_id, messages.subject, messages.preview,
                    messages.received_at, messages.seen, messages.flagged, messages.answered,
                    messages.draft, messages.has_attachments, messages.size,
                    (SELECT name FROM recipients
                      WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
                      ORDER BY recipients.position LIMIT 1),
                    (SELECT address FROM recipients
                      WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
                      ORDER BY recipients.position LIMIT 1)
               FROM ranked JOIN messages ON messages.id = ranked.id
              WHERE ranked.rank = 1",
            placeholders(ids.len(), 1)
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(ids.iter().map(|id| id.get())), |row| {
            // The column order matches the list row's, with thread_id inserted
            // second; read_list_row expects it there too.
            Ok((ThreadId::new(row.get(1)?), read_list_row(row)?))
        })?;

        let mut latest = HashMap::new();
        for row in rows {
            let (thread_id, message) = row?;
            latest.insert(thread_id, message);
        }
        Ok(latest)
    }

    fn mailboxes_in(&self, id: ThreadId) -> Result<Vec<MailboxId>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT DISTINCT mailbox_id FROM messages WHERE thread_id = ?1 AND {MEMBER}
              ORDER BY mailbox_id"
        ))?;
        let rows = statement.query_map([id.get()], |row| Ok(MailboxId::new(row.get(0)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn labels_in(&self, id: ThreadId) -> Result<Vec<LabelId>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT DISTINCT message_labels.label_id
               FROM message_labels
               JOIN messages ON messages.id = message_labels.message_id
              WHERE messages.thread_id = ?1 AND messages.{MEMBER}
              ORDER BY message_labels.label_id"
        ))?;
        let rows = statement.query_map([id.get()], |row| Ok(LabelId::new(row.get(0)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The thread a message is currently in.
fn thread_of(connection: &Connection, message_id: MessageId) -> Result<Option<ThreadId>> {
    let mut statement = connection.prepare("SELECT thread_id FROM messages WHERE id = ?1")?;
    let mut rows = statement.query([message_id.get()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(row.get::<_, Option<i64>>(0)?.map(ThreadId::new))
}

/// Recomputes one thread's aggregates from its members, in whatever
/// transaction the caller is already in.
///
/// The subject is the normalized subject of the oldest member — the message
/// that named the conversation — recomputed here because a merge can change
/// which message that is.
fn recompute_in(connection: &Connection, id: ThreadId) -> Result<()> {
    let root_subject: Option<String> = connection
        .query_row(
            &format!(
                "SELECT subject FROM messages WHERE thread_id = ?1 AND {MEMBER}
                  ORDER BY received_at, id LIMIT 1"
            ),
            [id.get()],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap_or(None);

    connection.execute(
        &format!(
            "UPDATE threads
                SET subject = coalesce(?2, subject),
                    message_count = (SELECT count(*) FROM messages
                                      WHERE thread_id = ?1 AND {MEMBER}),
                    unread_count = (SELECT count(*) FROM messages
                                     WHERE thread_id = ?1 AND {MEMBER} AND seen = 0),
                    has_attachments = EXISTS (SELECT 1 FROM messages
                                               WHERE thread_id = ?1 AND {MEMBER}
                                                 AND has_attachments = 1),
                    is_flagged = EXISTS (SELECT 1 FROM messages
                                          WHERE thread_id = ?1 AND {MEMBER} AND flagged = 1),
                    first_at = coalesce((SELECT min(received_at) FROM messages
                                          WHERE thread_id = ?1 AND {MEMBER}), 0),
                    last_at = coalesce((SELECT max(received_at) FROM messages
                                         WHERE thread_id = ?1 AND {MEMBER}), 0)
              WHERE id = ?1"
        ),
        params![id.get(), root_subject.as_deref().map(normalize_subject)],
    )?;
    Ok(())
}

fn read_thread(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: ThreadId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        subject: row.get(2)?,
        message_ids: Vec::new(),
        participants: Vec::new(),
        mailbox_ids: Vec::new(),
        labels: Vec::new(),
        message_count: row.get(3)?,
        unread_count: row.get(4)?,
        has_attachments: row.get(5)?,
        is_flagged: row.get(6)?,
        first_at: from_millis(row.get(7)?),
        last_at: from_millis(row.get(8)?),
    })
}
