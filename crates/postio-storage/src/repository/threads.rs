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
use crate::repository::MessageRepository;

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
    /// The row's recency: the sort key of the list this cursor resumes.
    pub last_at: DateTime<Utc>,
    /// The tiebreaker that makes the order total.
    ///
    /// Which id depends on what the list is ordered over — the thread's in an
    /// account-scoped list, the representative message's in a folder-scoped
    /// one. It is only ever compared against the same column it came from, so
    /// it is carried as the integer it is rather than pretending to be one
    /// kind of id in both.
    pub id: i64,
}

/// One window of the thread list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListQuery {
    /// Whose conversations. Threads never span accounts.
    pub account_id: AccountId,
    /// The folder the list is showing, if it is showing one.
    ///
    /// A real folder threads (ADR 0015): only conversations holding a message
    /// here appear, the row is drawn from the newest message *here*, and the
    /// unread count is this folder's slice. `None` is the whole account,
    /// which is what the unified list and the drill-in read.
    ///
    /// The total message count is never scoped — the badge means the size of
    /// the conversation, wherever it is filed.
    pub mailbox: Option<MailboxId>,
    /// How many rows at most.
    pub limit: u32,
    /// Where to resume; `None` starts at the most recently active thread.
    pub after: Option<ThreadCursor>,
}

/// One window of the unified list: every account, newest first.
#[derive(Debug, Clone, Copy)]
pub struct UnifiedThreadListQuery {
    /// How many groups at most.
    pub limit: u32,
    /// Where to resume — the cursor of the last group drawn.
    pub after: Option<ThreadCursor>,
}

/// One unified-list row: a conversation, wherever the user received it.
///
/// See [`ThreadRepository::unified_page`]. `row` is what the list draws;
/// `members` is what an action expands to — one thread per account holding
/// a copy, so "archive" means two operations in two per-account queues,
/// which is the only answer that matches what the user believes they did
/// (ADR 0005 Q2).
#[derive(Debug, Clone)]
pub struct ThreadGroup {
    /// The row the list draws, counts deduped across members.
    pub row: ThreadListRow,
    /// Every copy of the conversation: `(account, thread)` pairs.
    pub members: Vec<(AccountId, ThreadId)>,
}

impl ThreadGroup {
    /// Where the next page resumes after this group.
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            last_at: self.row.last_at,
            id: self.row.sort_id,
        }
    }
}

/// `THREAD_COLUMNS`, each qualified with `alias.` for a joined statement.
fn prefixed_thread_columns(alias: &str) -> String {
    THREAD_COLUMNS
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ThreadListQuery {
    /// Every thread in an account.
    pub fn account(account_id: AccountId) -> Self {
        Self {
            account_id,
            mailbox: None,
            limit: DEFAULT_THREAD_PAGE_SIZE,
            after: None,
        }
    }

    /// The conversations a folder holds a message of.
    pub fn in_mailbox(account_id: AccountId, mailbox: MailboxId) -> Self {
        Self {
            account_id,
            mailbox: Some(mailbox),
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
    /// The conversation this row stands for.
    ///
    /// `None` for a message that belongs to no thread. Threading runs on
    /// every message sync files, but it can fail — `postio-sync`'s send path
    /// ignores the result outright — and **a list that hides mail because a
    /// derived column is null is worse than a list that shows it ungrouped**.
    /// So an unthreaded message is a conversation of one rather than a row
    /// that does not exist.
    pub id: Option<ThreadId>,
    /// Normalized subject of the conversation's root message.
    pub subject: Option<String>,
    /// Everyone who has written in the thread, in first-seen order.
    pub participants: Vec<EmailAddress>,
    /// How many messages the conversation holds, across every folder.
    ///
    /// Never scoped, even in a folder-scoped query: the badge means the size
    /// of the conversation (ADR 0015 Q2).
    pub message_count: u32,
    /// How many are unread **within the query's scope**.
    ///
    /// In a folder that is this folder's slice, so a conversation whose only
    /// unread member is filed elsewhere reads as handled here.
    pub unread_count: u32,
    /// Whether any member carries an attachment.
    pub has_attachments: bool,
    /// Whether any member **within the query's scope** is flagged.
    pub is_flagged: bool,
    /// When the conversation started.
    pub first_at: DateTime<Utc>,
    /// When it last moved; the sort key.
    pub last_at: DateTime<Utc>,
    /// The newest message **within the query's scope**, which is what the
    /// row's snippet and sender come from — a reply filed in Archive is not
    /// what the Inbox row should be showing.
    ///
    /// `None` only for a thread whose messages have all been hidden.
    pub latest: Option<MessageListRow>,
    /// The tiebreaker this row sorts by, for [`ThreadListRow::cursor`].
    ///
    /// The thread's id in an account-scoped list, the representative
    /// message's in a folder-scoped one — the two windows are ordered over
    /// different columns, and a cursor is only ever compared against the one
    /// it came from.
    pub sort_id: i64,
}

impl ThreadListRow {
    /// The cursor that resumes paging immediately after this row.
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            last_at: self.last_at,
            id: self.sort_id,
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
        let transaction = super::Scope::open(self.connection)?;
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
        let transaction = super::Scope::open(self.connection)?;
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
        let transaction = super::Scope::open(self.connection)?;
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

    /// One window of the unified thread list: every account, grouped at
    /// read time (#184, ADR 0005 Q2).
    ///
    /// A thread never spans accounts — that is sync state, and it stays
    /// per-account. What the unified list shows is a [`ThreadGroup`]:
    /// threads from different accounts folded into one row when the
    /// conversation is one conversation to the *user*. Two rules, in
    /// order:
    ///
    /// - **Root identity.** Another account holds a copy of this thread's
    ///   JWZ root (same `RfcMessageId`), looked up over
    ///   `idx_messages_rfc_message_id`.
    /// - **Subject, within the window.** Neither thread has a root id to
    ///   compare — common — but their normalised subjects match and their
    ///   activity is within [`postio_model::subject::COALESCING_WINDOW_DAYS`].
    ///   A bare subject match with no window would fold every "Weekly
    ///   digest" the user receives at two addresses into one eternal row.
    ///
    /// The page walks threads newest-first over `idx_threads_last_at` and
    /// emits a group only at its **newest** member — an older partner is
    /// absorbed, on this page or any later one, so no conversation is ever
    /// two rows. Dedupe is display-only (Q13): `message_count` counts
    /// distinct `RfcMessageId`s across the members, both copies stay, and
    /// [`ThreadGroup::members`] is exactly what an action must expand to.
    pub fn unified_page(&self, query: &UnifiedThreadListQuery) -> Result<Vec<ThreadGroup>> {
        let mut groups: Vec<ThreadGroup> = Vec::new();
        let mut absorbed: std::collections::HashSet<ThreadId> = std::collections::HashSet::new();
        let mut cursor = query.after;

        // The raw page over-fetches: absorption folds rows together, so a
        // page of threads can under-fill the page of groups. Loop until the
        // groups fill or the list ends; each pass is one indexed window.
        'fill: loop {
            let raw = self.unified_raw_page(query.limit.max(2) * 2, cursor)?;
            let Some(last) = raw.last() else {
                break;
            };
            cursor = Some(ThreadCursor {
                last_at: last.last_at,
                id: last.id.get(),
            });
            let exhausted = raw.len() < (query.limit.max(2) * 2) as usize;

            let mut partner_map = self.group_partners_for(&raw)?;
            for thread in raw {
                if absorbed.contains(&thread.id) {
                    continue;
                }
                let partners = partner_map.remove(&thread.id).unwrap_or_default();
                // A partner newer than this thread means this is not the
                // group's head: the head already drew the row (this page or
                // an earlier one), or will when the walk reaches it — it
                // cannot, because the walk is newest-first; it already did.
                if partners.iter().any(|partner| {
                    (partner.last_at, partner.id.get()) > (thread.last_at, thread.id.get())
                }) {
                    continue;
                }
                for partner in &partners {
                    absorbed.insert(partner.id);
                }

                let row = self.group_row(&thread, &partners)?;
                let members = std::iter::once((thread.account_id, thread.id))
                    .chain(
                        partners
                            .iter()
                            .map(|partner| (partner.account_id, partner.id)),
                    )
                    .collect();
                groups.push(ThreadGroup { row, members });
                if groups.len() as u32 >= query.limit {
                    break 'fill;
                }
            }
            if exhausted {
                break;
            }
        }

        // Two reads for the whole page rather than two per group — the same
        // batching `page` does, for the same reason.
        let heads: Vec<ThreadId> = groups.iter().filter_map(|group| group.row.id).collect();
        let mut participants = self.participants_for(&heads)?;
        let mut latest = self.latest_messages_for(&heads, None)?;
        for group in &mut groups {
            if let Some(id) = group.row.id {
                group.row.participants = participants.remove(&id).unwrap_or_default();
                group.row.latest = latest.remove(&id);
            }
        }
        Ok(groups)
    }

    /// One raw window of threads across every account, newest first.
    fn unified_raw_page(&self, limit: u32, after: Option<ThreadCursor>) -> Result<Vec<Thread>> {
        let cursor = if after.is_some() {
            " AND (last_at, id) < (?1, ?2)"
        } else {
            ""
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT {THREAD_COLUMNS} FROM threads
              WHERE message_count > 0{cursor}
              ORDER BY last_at DESC, id DESC LIMIT {limit}"
        ))?;
        let mut arguments: Vec<i64> = Vec::new();
        if let Some(after) = after {
            arguments.push(to_millis(after.last_at));
            arguments.push(after.id);
        }
        let rows = statement.query_map(params_from_iter(arguments), read_thread)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The threads in *other* accounts that are each page thread's
    /// conversation — by root identity, then by subject within the window.
    ///
    /// Three statements for the whole page, not three per thread: the
    /// per-thread version was the unified page's entire cost.
    fn group_partners_for(&self, page: &[Thread]) -> Result<HashMap<ThreadId, Vec<Thread>>> {
        let mut partners: HashMap<ThreadId, Vec<Thread>> = HashMap::new();
        let mut seen: HashMap<ThreadId, std::collections::HashSet<ThreadId>> = HashMap::new();
        if page.is_empty() {
            return Ok(partners);
        }
        let ids: Vec<i64> = page.iter().map(|thread| thread.id.get()).collect();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");

        // 1. Every page thread's root RfcMessageId, in one window pass.
        let mut roots: HashMap<String, Vec<&Thread>> = HashMap::new();
        {
            let mut statement = self.connection.prepare(&format!(
                "SELECT thread_id, rfc_message_id FROM (
                     SELECT thread_id, rfc_message_id,
                            row_number() OVER (PARTITION BY thread_id
                                               ORDER BY received_at, id) AS rank
                       FROM messages
                      WHERE thread_id IN ({placeholders}) AND {MEMBER}
                 ) WHERE rank = 1"
            ))?;
            let rows = statement.query_map(params_from_iter(&ids), |row| {
                Ok((
                    ThreadId::new(row.get::<_, i64>(0)?),
                    row.get::<_, Option<String>>(1)?,
                ))
            })?;
            for row in rows {
                let (thread_id, root) = row?;
                if let (Some(root), Some(thread)) = (
                    root.filter(|root| !root.is_empty()),
                    page.iter().find(|thread| thread.id == thread_id),
                ) {
                    roots.entry(root).or_default().push(thread);
                }
            }
        }

        // 2. Partners by root identity, over idx_messages_rfc_message_id.
        if !roots.is_empty() {
            let root_keys: Vec<&String> = roots.keys().collect();
            let root_placeholders = std::iter::repeat_n("?", root_keys.len())
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = self.connection.prepare(&format!(
                "SELECT DISTINCT m.rfc_message_id, {columns} FROM threads t
                   JOIN messages m ON m.thread_id = t.id
                  WHERE m.rfc_message_id IN ({root_placeholders})
                    AND t.message_count > 0",
                columns = prefixed_thread_columns("t")
            ))?;
            let rows = statement.query_map(params_from_iter(&root_keys), |row| {
                let root: String = row.get(0)?;
                let mut candidate = read_thread_offset(row, 1)?;
                candidate.message_ids = Vec::new();
                Ok((root, candidate))
            })?;
            for row in rows {
                let (root, candidate) = row?;
                for thread in roots.get(root.as_str()).into_iter().flatten() {
                    if candidate.account_id != thread.account_id
                        && seen.entry(thread.id).or_default().insert(candidate.id)
                    {
                        partners
                            .entry(thread.id)
                            .or_default()
                            .push(candidate.clone());
                    }
                }
            }
        }

        // 3. Partners by subject, inside the coalescing window.
        let subjects: Vec<&str> = page
            .iter()
            .filter_map(|thread| thread.subject.as_deref())
            .filter(|subject| !subject.is_empty())
            .collect();
        if !subjects.is_empty() {
            let window_millis =
                postio_model::subject::COALESCING_WINDOW_DAYS * 24 * 60 * 60 * 1_000;
            let subject_placeholders = std::iter::repeat_n("?", subjects.len())
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = self.connection.prepare(&format!(
                "SELECT {THREAD_COLUMNS} FROM threads
                  WHERE subject IN ({subject_placeholders}) AND message_count > 0"
            ))?;
            let rows = statement.query_map(params_from_iter(&subjects), read_thread)?;
            for candidate in rows {
                let candidate = candidate?;
                for thread in page {
                    if thread.subject.as_deref() == candidate.subject.as_deref()
                        && candidate.account_id != thread.account_id
                        && (to_millis(candidate.last_at) - to_millis(thread.last_at)).abs()
                            <= window_millis
                        && seen.entry(thread.id).or_default().insert(candidate.id)
                    {
                        partners
                            .entry(thread.id)
                            .or_default()
                            .push(candidate.clone());
                    }
                }
            }
        }
        Ok(partners)
    }

    /// The group's display row: the head thread's row, counts deduped
    /// across every member.
    ///
    /// Participants and the latest message are filled by the caller in one
    /// batched read per page, the same way [`ThreadRepository::page`] does —
    /// per-group reads were most of a page's cost.
    fn group_row(&self, head: &Thread, partners: &[Thread]) -> Result<ThreadListRow> {
        let mut row = ThreadListRow {
            id: Some(head.id),
            subject: head.subject.clone(),
            participants: Vec::new(),
            message_count: head.message_count,
            unread_count: head.unread_count,
            has_attachments: head.has_attachments,
            is_flagged: head.is_flagged,
            first_at: head.first_at,
            last_at: head.last_at,
            latest: None,
            sort_id: head.id.get(),
        };
        if partners.is_empty() {
            // The overwhelmingly common group: one thread, one account. Its
            // own maintained counts are already the answer, and asking SQL
            // to dedupe a set of one was most of the unified page's cost.
            return Ok(row);
        }

        // Distinct messages, not distinct rows: a copy received at two
        // addresses is one message to the user. A message with no
        // RfcMessageId can never be anyone's copy, so it counts by row.
        let mut members: Vec<i64> = vec![head.id.get()];
        members.extend(partners.iter().map(|partner| partner.id.get()));
        let placeholders = std::iter::repeat_n("?", members.len())
            .collect::<Vec<_>>()
            .join(", ");
        let (message_count, unread_count): (u32, u32) = self.connection.query_row(
            &format!(
                "SELECT
                     count(DISTINCT coalesce(nullif(m.rfc_message_id, ''), 'row:' || m.id)),
                     count(DISTINCT CASE WHEN m.seen = 0
                         THEN coalesce(nullif(m.rfc_message_id, ''), 'row:' || m.id) END)
                   FROM messages m
                  WHERE m.thread_id IN ({placeholders}) AND m.{MEMBER}"
            ),
            params_from_iter(&members),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        row.message_count = message_count;
        row.unread_count = unread_count;
        for partner in partners {
            row.has_attachments |= partner.has_attachments;
            row.is_flagged |= partner.is_flagged;
            row.first_at = row.first_at.min(partner.first_at);
        }
        Ok(row)
    }

    /// One window of the thread list, most recently active first.
    pub fn page(&self, query: &ThreadListQuery) -> Result<Vec<ThreadListRow>> {
        self.page_with(query, "")
    }

    /// [`ThreadRepository::page`] with `tail` appended to the statement.
    fn page_with(&self, query: &ThreadListQuery, tail: &str) -> Result<Vec<ThreadListRow>> {
        let mut statement = self
            .connection
            .prepare(&format!("{}{tail}", self.explain(query)))?;
        let mut arguments = vec![query.account_id.get()];
        // `?2` when the query is folder-scoped, so the cursor follows at ?3/?4
        // rather than ?2/?3 — `explain` numbers them the same way.
        if let Some(mailbox) = query.mailbox {
            arguments.push(mailbox.get());
        }
        if let Some(cursor) = query.after {
            arguments.push(to_millis(cursor.last_at));
            arguments.push(cursor.id);
        }
        let scoped = query.mailbox.is_some();
        let rows = statement.query_map(params_from_iter(arguments), |row| {
            let thread = row.get::<_, i64>(0)?;
            Ok((
                ThreadListRow {
                    // Zero is the folder window's spelling of "no thread": an
                    // id column cannot be null and still be compared, so the
                    // query coalesces and this un-coalesces.
                    id: (thread != 0).then(|| ThreadId::new(thread)),
                    subject: row.get(2)?,
                    participants: Vec::new(),
                    message_count: row.get(3)?,
                    unread_count: row.get(4)?,
                    has_attachments: row.get(5)?,
                    is_flagged: row.get(6)?,
                    first_at: from_millis(row.get(7)?),
                    last_at: from_millis(row.get(8)?),
                    latest: None,
                    sort_id: if scoped { row.get(9)? } else { thread },
                },
                // The representative's id, which the folder window already
                // knows and the account window has to look up.
                scoped.then(|| MessageId::new(row.get::<_, i64>(9).unwrap_or_default())),
            ))
        })?;
        let mut page: Vec<(ThreadListRow, Option<MessageId>)> = rows.collect::<Result<_, _>>()?;
        drop(statement);

        // Two more statements for the whole page, rather than two per row.
        let ids: Vec<ThreadId> = page.iter().filter_map(|(row, _)| row.id).collect();
        let mut participants = self.participants_for(&ids)?;
        if scoped {
            // The window already named the representative of every row, so
            // this is one read by id rather than a window function over the
            // conversations.
            let wanted: Vec<MessageId> = page.iter().filter_map(|(_, id)| *id).collect();
            let mut latest: HashMap<MessageId, MessageListRow> =
                MessageRepository::new(self.connection)
                    .rows_for(&wanted)?
                    .into_iter()
                    .map(|row| (row.id, row))
                    .collect();
            for (row, representative) in &mut page {
                if let Some(id) = row.id {
                    row.participants = participants.remove(&id).unwrap_or_default();
                }
                row.latest = representative.and_then(|id| latest.remove(&id));
                // A conversation of one has one participant: whoever wrote
                // it. Without this an unthreaded message would draw a blank
                // sender line.
                if row.participants.is_empty()
                    && let Some(from) = row.latest.as_ref().and_then(|row| row.from.clone())
                {
                    row.participants = vec![from];
                }
            }
        } else {
            let mut latest = self.latest_messages_for(&ids, None)?;
            for (row, _) in &mut page {
                if let Some(id) = row.id {
                    row.participants = participants.remove(&id).unwrap_or_default();
                    row.latest = latest.remove(&id);
                }
            }
        }
        Ok(page.into_iter().map(|(row, _)| row).collect())
    }

    /// The SQL a thread page runs, for `EXPLAIN QUERY PLAN`.
    ///
    /// # Why the folder-scoped shape is still flat
    ///
    /// The window is over `threads`, ordered by `last_at DESC, id DESC` over
    /// `idx_threads_account_last_at` — the same seek the unscoped list makes,
    /// so SQLite never sorts and never scans the table. Everything the folder
    /// contributes is a **correlated subquery per row of the page**, not a
    /// join that widens the set being ordered: whether the folder holds any
    /// of the conversation, how much of it is unread here, and whether any of
    /// it is flagged here. Each of those seeks
    /// `idx_messages_thread_mailbox (thread_id, mailbox_id, received_at DESC,
    /// id DESC)`, which migration 0012 added for exactly this, and each is
    /// bounded by the size of one conversation rather than by the mailbox.
    ///
    /// So a page costs `limit` index seeks plus a constant per row, whatever
    /// the folder holds — which is what "page k of threads costs what page k
    /// of messages costs" means. `the_thread_list_plan_never_sorts` is the
    /// structural half of that claim and `store_reads` is the empirical half.
    pub fn explain(&self, query: &ThreadListQuery) -> String {
        // `message_count > 0` hides a conversation whose messages have all been
        // hidden: an empty row is not something the user can act on.
        let Some(_) = query.mailbox else {
            let cursor = if query.after.is_some() {
                " AND (last_at, id) < (?2, ?3)"
            } else {
                ""
            };
            return format!(
                "SELECT {THREAD_COLUMNS} FROM threads
                  WHERE account_id = ?1 AND message_count > 0{cursor}
                  ORDER BY last_at DESC, id DESC LIMIT {}",
                query.limit
            );
        };

        let cursor = if query.after.is_some() {
            " AND (rep.received_at, rep.id) < (?3, ?4)"
        } else {
            ""
        };
        // The folder's slice of this row's conversation. Spelled once and
        // reused, so the aggregates cannot drift apart on what counts as a
        // member here.
        let slice = format!(
            "FROM messages m
              WHERE m.thread_id = rep.thread_id AND m.mailbox_id = ?2 AND m.{MEMBER}"
        );
        format!(
            "SELECT coalesce(rep.thread_id, 0), ?1, rep.subject,
                    coalesce((SELECT t.message_count FROM threads t
                               WHERE t.id = rep.thread_id), 1),
                    coalesce((SELECT count(*) {slice} AND m.seen = 0),
                             CASE WHEN rep.seen = 0 THEN 1 ELSE 0 END),
                    coalesce((SELECT max(m.has_attachments) {slice}), rep.has_attachments),
                    coalesce((SELECT max(m.flagged) {slice}), rep.flagged),
                    rep.received_at, rep.received_at, rep.id
               FROM messages rep
              WHERE rep.mailbox_id = ?2 AND rep.{MEMBER}
                AND NOT EXISTS (
                        SELECT 1 FROM messages newer
                         WHERE newer.mailbox_id = ?2 AND newer.{MEMBER}
                           AND newer.thread_id IS NOT NULL
                           AND newer.thread_id = rep.thread_id
                           AND (newer.received_at, newer.id) > (rep.received_at, rep.id)
                    ){cursor}
              ORDER BY rep.received_at DESC, rep.id DESC LIMIT {}",
            query.limit
        )
    }

    /// One window of the thread list at a row offset, for a list model that
    /// scrolls by index.
    ///
    /// Prefer [`ThreadRepository::page`]: an offset is counted from the top of
    /// the list every time, so a conversation that moves while the user
    /// scrolls shifts every row down and this window silently skips one. The
    /// store's seek marks exist to keep the offset small — see
    /// `postio_runtime::store`.
    pub fn page_at(&self, query: &ThreadListQuery, offset: u32) -> Result<Vec<ThreadListRow>> {
        if offset == 0 {
            return self.page(query);
        }
        self.page_with(query, &format!(" OFFSET {offset}"))
    }

    /// How many threads the list would show.
    pub fn count(&self, account_id: AccountId) -> Result<u32> {
        self.count_of(&ThreadListQuery::account(account_id))
    }

    /// How many threads `query`'s scope would show.
    ///
    /// The folder-scoped count is the same `EXISTS` the page uses, so the
    /// number and the rows cannot disagree about what "in this folder" means.
    pub fn count_of(&self, query: &ThreadListQuery) -> Result<u32> {
        let count: i64 = match query.mailbox {
            None => self.connection.query_row(
                "SELECT count(*) FROM threads WHERE account_id = ?1 AND message_count > 0",
                [query.account_id.get()],
                |row| row.get(0),
            )?,
            // The same predicate the window uses, so the number and the rows
            // cannot disagree about what a row is: one per conversation the
            // folder holds, plus one per message it holds that belongs to no
            // conversation.
            Some(mailbox) => self.connection.query_row(
                &format!(
                    "SELECT count(*) FROM messages rep
                      WHERE rep.mailbox_id = ?1 AND rep.{MEMBER}
                        AND NOT EXISTS (
                                SELECT 1 FROM messages newer
                                 WHERE newer.mailbox_id = ?1 AND newer.{MEMBER}
                                   AND newer.thread_id IS NOT NULL
                                   AND newer.thread_id = rep.thread_id
                                   AND (newer.received_at, newer.id)
                                       > (rep.received_at, rep.id))"
                ),
                [mailbox.get()],
                |row| row.get(0),
            )?,
        };
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
            "SELECT messages.thread_id, recipients.name, addresses.address,
                    min(messages.received_at) AS first_seen
               FROM messages
               JOIN recipients ON recipients.message_id = messages.id
               JOIN addresses ON addresses.id = recipients.address_id
              WHERE messages.thread_id IN ({}) AND messages.{MEMBER}
                AND recipients.kind = 'from'
              GROUP BY messages.thread_id, recipients.address_id
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
    fn latest_messages_for(
        &self,
        ids: &[ThreadId],
        mailbox: Option<MailboxId>,
    ) -> Result<HashMap<ThreadId, MessageListRow>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        // Still one statement for the whole page: the folder narrows what the
        // window function ranks, it does not turn this into a query per row.
        let scope = match mailbox {
            Some(mailbox) => format!(" AND mailbox_id = {}", mailbox.get()),
            None => String::new(),
        };
        let sql = format!(
            "WITH ranked AS (
                 SELECT id, thread_id, subject, preview, received_at, seen, flagged, answered,
                        draft, has_attachments, size,
                        row_number() OVER (PARTITION BY thread_id
                                           ORDER BY received_at DESC, id DESC) AS rank
                   FROM messages
                  WHERE thread_id IN ({}) AND {MEMBER}{scope}
             )
             SELECT messages.id, messages.thread_id, messages.subject, messages.preview,
                    messages.received_at, messages.seen, messages.flagged, messages.answered,
                    messages.draft, messages.has_attachments, messages.size,
                    (SELECT name FROM recipients
                      WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
                      ORDER BY recipients.position LIMIT 1),
                    (SELECT addresses.address FROM recipients
                        JOIN addresses ON addresses.id = recipients.address_id
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

/// [`read_thread`], with the thread's columns starting at `offset`.
fn read_thread_offset(row: &Row<'_>, offset: usize) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: ThreadId::new(row.get(offset)?),
        account_id: AccountId::new(row.get(offset + 1)?),
        subject: row.get(offset + 2)?,
        message_ids: Vec::new(),
        participants: Vec::new(),
        mailbox_ids: Vec::new(),
        labels: Vec::new(),
        message_count: row.get(offset + 3)?,
        unread_count: row.get(offset + 4)?,
        has_attachments: row.get(offset + 5)?,
        is_flagged: row.get(offset + 6)?,
        first_at: from_millis(row.get(offset + 7)?),
        last_at: from_millis(row.get(offset + 8)?),
    })
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
