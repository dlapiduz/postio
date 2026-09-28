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
use postio_model::listing::{MarkerSummary, MarkerWhen};
use postio_model::{
    AccountId, EmailAddress, LabelId, MailboxId, MessageId, Thread, ThreadId, normalize_subject,
};

use super::messages::{
    LIST_COLUMNS, MessageListRow, list_columns_of, placeholders, read_list_row, read_list_row_at,
};
use super::{from_millis, require_persisted, to_millis};

use crate::error::{Error, Result};
use crate::repository::MessageRepository;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

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

/// One window of the unified list: every enabled account's inbox, newest
/// first.
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// `alias`'s thread belongs to an account the user has switched on.
///
/// The unified view is every *enabled* account's inbox, and ADR 0005 Q10 is
/// explicit that this is the one omission that needs no disclosure: a
/// disabled account drops out silently and correctly, because the user asked
/// for that. It is a failing account that has to be named, and that is a
/// different axis entirely.
///
/// Applied to the partner search as well as to the page, and that is the half
/// that is easy to miss: a thread in a disabled account that absorbed its
/// partner in an enabled one would take a row the user can still see and fold
/// it into a row they cannot.
fn enabled_account(alias: &str) -> String {
    format!(
        "{alias}account_id IN \
         (SELECT id FROM accounts WHERE enabled = 1 AND pending_deletion = 0)"
    )
}

/// A row's `(recency, id)`, the order every list here sorts by, newest
/// last -- compared the way the cursor's `<=`/`<` pair compares it.
fn sort_key(cursor: ThreadCursor) -> (DateTime<Utc>, i64) {
    (cursor.last_at, cursor.id)
}

/// How many inbox conversations [`ThreadRepository::unified_count`] asks
/// about partners for at once: three statements per batch, with an `IN`
/// list well under the engine's parameter limit.
const COUNT_BATCH: usize = 400;

/// A thread in another account that is the same conversation as a unified
/// row, and is itself in view: `at` is where its own inbox draws it.
#[derive(Debug, Clone)]
struct Partner {
    thread: Thread,
    at: ThreadCursor,
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

/// One window of Focus's inbox (spec 007): every inbox in `inboxes`, one
/// row per conversation it holds, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusListQuery {
    /// The inboxes the list is made of, as
    /// [`ThreadRepository::unified_inboxes`] names them.
    pub inboxes: Vec<(AccountId, MailboxId)>,
    /// How many rows at most.
    pub limit: u32,
    /// Where to resume; `None` starts at the newest conversation.
    pub after: Option<ThreadCursor>,
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
    /// What Focus calls out on the row: the conversation's newest marker
    /// the person has not dismissed (spec 007).
    ///
    /// Read by Focus's pages only, in one statement for the page; every
    /// list the classic app and the terminal read leaves it `None` and
    /// reads nothing to find that out.
    pub marker: Option<MarkerSummary>,
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
/// A message hidden pending a remote delete, or snoozed and not yet due, is
/// not one: the list does not show it, so it must not be in the count, in
/// the drill-in or in the participants. Every call site interpolates this
/// as `{alias}.{MEMBER}` or bare `{MEMBER}`, so the snooze half is written
/// without an alias of its own — `snoozed_until` names no other table in
/// any query here, so it resolves the same way regardless.
///
/// The same predicate the folder counts keep, so the list and its counts
/// cannot drift apart: [`super::VISIBLE`].
const MEMBER: &str = super::VISIBLE;

/// Which of a folder's messages are rows of a list over it: the membership
/// test, asked in one place by every list query here (spec 007, research
/// R13).
///
/// Every list the classic app and the terminal read asks
/// [`Membership::Folder`], which is [`MEMBER`] and nothing else. Focus's
/// inbox asks [`Membership::Focus`]: the same test with
/// [`super::focus_excludes`] after it, which is empty until Focus holds mail
/// back and is then the one predicate that takes held mail out of the
/// window, the representative's `NOT EXISTS`, the slice, the count, the seek
/// marks and the rows for changed messages together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Membership {
    /// A folder's own list, and the unified inbox made of them.
    Folder,
    /// Focus's inbox.
    Focus,
}

impl Membership {
    /// The test for the message `alias` names: `"rep."`, `"m."`, or `""`
    /// for a statement's only `messages`.
    fn test(self, alias: &str) -> String {
        format!("{alias}{MEMBER}{}", self.excludes(alias))
    }

    /// What this membership leaves out beyond [`MEMBER`], as a conjunct
    /// (` AND ...`) or nothing: for a statement that asks [`MEMBER`] of a
    /// wider set and this membership of part of it, as a folded row's
    /// unread count does.
    fn excludes(self, alias: &str) -> String {
        match self {
            Membership::Folder => String::new(),
            Membership::Focus => super::focus_excludes(alias),
        }
    }
}

/// How many rows a folder's thread list has: one per conversation the
/// folder holds, plus one per message it holds that belongs to no
/// conversation -- the same predicate the window uses, so the number and the
/// rows cannot disagree about what a row is.
fn folder_count_sql(membership: Membership) -> String {
    // Distinct conversations, with a lone message standing for itself under
    // its negated id, which no conversation id can equal. Read entirely
    // from `idx_messages_mailbox_threads`: the old shape asked, for every
    // message, whether a newer member of its conversation was here too --
    // 786 ms on a real 60,907-message folder (#1534, #1607).
    format!(
        "SELECT count(DISTINCT coalesce(thread_id, -id)) FROM messages
          WHERE mailbox_id = ?1 AND {}",
        membership.test("")
    )
}

/// The rows of `mailbox`'s list: a message there that no newer member of
/// its conversation there outranks -- the representative the window draws a
/// row from. `mailbox` is the placeholder that names the folder, and `also`
/// narrows the candidates before the `NOT EXISTS` is asked of them.
fn representative_filter(membership: Membership, mailbox: &str, also: &str) -> String {
    format!(
        "rep.mailbox_id = {mailbox} AND {}{also}
                AND NOT EXISTS (
                        SELECT 1 FROM messages newer
                         WHERE newer.mailbox_id = {mailbox} AND {}
                           AND newer.thread_id IS NOT NULL
                           AND newer.thread_id = rep.thread_id
                           AND (newer.sort_at, newer.id) > (rep.sort_at, rep.id)
                    )",
        membership.test("rep."),
        membership.test("newer.")
    )
}

/// The folder's slice of a representative's conversation, for the row's
/// aggregates: spelled once, so they cannot drift apart on what counts as a
/// member here.
fn slice(membership: Membership, mailbox: &str) -> String {
    format!(
        "FROM messages m
              WHERE m.thread_id = rep.thread_id AND m.mailbox_id = {mailbox} AND {}",
        membership.test("m.")
    )
}

/// The conversations of Focus's inbox that draw a marker (spec 007 T048):
/// the has-action filter's rows, as conversations and as messages in none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marked {
    /// Conversations with a marker the person has not dismissed.
    pub threads: Vec<ThreadId>,
    /// Messages in no conversation that carry one.
    pub lone: Vec<MessageId>,
}

impl Marked {
    /// How many rows the has-action filter shows.
    pub fn len(&self) -> u32 {
        (self.threads.len() + self.lone.len()) as u32
    }

    /// Whether it shows none.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.lone.is_empty()
    }
}

/// Where Focus's window starts reading the representative's list row: after
/// the ten columns every thread window has.
const FOCUS_REPRESENTATIVE: usize = 10;

/// One inbox's part of Focus's window: the folder window over `?{mailbox}`,
/// asking Focus's membership, with the representative's list row read in
/// the same statement rather than by id afterwards.
///
/// The sort key and id are named, so a merge of several of these can order
/// by them.
fn focus_arm(mailbox: usize, cursor: Option<(usize, usize)>, limit: u32) -> String {
    let membership = Membership::Focus;
    let mailbox = format!("?{mailbox}");
    let slice = slice(membership, &mailbox);
    let cursor = cursor
        .map(|(at, id)| {
            format!(" AND rep.sort_at <= ?{at} AND (rep.sort_at < ?{at} OR rep.id < ?{id})")
        })
        .unwrap_or_default();
    format!(
        "SELECT coalesce(rep.thread_id, 0), rep.account_id, rep.subject,
                coalesce((SELECT t.message_count FROM threads t
                           WHERE t.id = rep.thread_id), 1),
                coalesce((SELECT count(*) {slice} AND m.seen = 0),
                         CASE WHEN rep.seen = 0 THEN 1 ELSE 0 END),
                coalesce((SELECT max(m.has_attachments) {slice}), rep.has_attachments),
                coalesce((SELECT max(m.flagged) {slice}), rep.flagged),
                rep.received_at, rep.sort_at AS focus_at, rep.id AS focus_id,
                {representative}
           FROM messages rep
          WHERE {filter}{cursor}
          ORDER BY rep.sort_at DESC, rep.id DESC LIMIT {limit}",
        representative = list_columns_of("rep"),
        filter = representative_filter(membership, &mailbox, ""),
    )
}

/// A row of Focus's window: the account whose inbox it is in, the
/// conversation, and its representative.
fn read_focus_row(row: &Row) -> Result<(AccountId, ThreadListRow)> {
    let thread = row.col::<i64>(0)?;
    let account = AccountId::new(row.col(1)?);
    Ok((
        account,
        ThreadListRow {
            // Zero is the window's spelling of "no thread", as in the folder's.
            id: (thread != 0).then(|| ThreadId::new(thread)),
            subject: row.col(2)?,
            participants: Vec::new(),
            message_count: row.col(3)?,
            unread_count: row.col(4)?,
            has_attachments: row.col(5)?,
            is_flagged: row.col(6)?,
            first_at: from_millis(row.col(7)?),
            last_at: from_millis(row.col(8)?),
            latest: Some(read_list_row_at(row, FOCUS_REPRESENTATIVE)?),
            sort_id: row.col(9)?,
            marker: None,
        },
    ))
}

impl<'a> ThreadRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a thread, assigning its id.
    ///
    /// The aggregates are whatever the value carries; they become true once
    /// messages are added, because every mutation here recomputes them.
    pub async fn create(&self, thread: &mut Thread) -> Result<ThreadId> {
        let account_id = require_persisted(thread.account_id.get(), "account")?;
        // Cached: a first sync creates a thread for most messages it files, so
        // this runs on the same order as the message insert itself (#728).
        sql::statement(
            self.connection,
            "INSERT INTO threads (account_id, subject, message_count, unread_count,
                                  has_attachments, is_flagged, first_at, last_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .await?
        .execute(bind![
            account_id,
            thread.subject,
            thread.message_count,
            thread.unread_count,
            thread.has_attachments,
            thread.is_flagged,
            to_millis(thread.first_at),
            to_millis(thread.last_at),
        ])
        .await?;
        thread.id = ThreadId::new(self.connection.last_insert_rowid());
        Ok(thread.id)
    }

    /// Writes a thread's aggregates back.
    ///
    /// Membership is not part of this: a message joins a thread through
    /// [`ThreadRepository::add_message`], never by being listed here.
    pub async fn update(&self, thread: &Thread) -> Result<()> {
        let id = require_persisted(thread.id.get(), "thread")?;
        let changed = sql::execute(
            self.connection,
            "UPDATE threads
                SET account_id = ?2, subject = ?3, message_count = ?4, unread_count = ?5,
                    has_attachments = ?6, is_flagged = ?7, first_at = ?8, last_at = ?9
              WHERE id = ?1",
            bind![
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
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "thread",
                id,
            });
        }
        Ok(())
    }

    /// One thread, with its membership and participants derived.
    pub async fn get(&self, id: ThreadId) -> Result<Option<Thread>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1"),
        )
        .await?;
        let found = crate::sql::first_of(&mut statement, [id.get()], read_thread).await?;
        let Some(mut thread) = found else {
            return Ok(None);
        };
        drop(statement);

        thread.message_ids = self.member_ids(id).await?;
        thread.participants = self
            .participants_for(&[id])
            .await?
            .remove(&id)
            .unwrap_or_default();
        thread.mailbox_ids = self.mailboxes_in(id).await?;
        thread.labels = self.labels_in(id).await?;
        Ok(Some(thread))
    }

    /// Deletes a thread, returning whether there was one.
    ///
    /// Its messages survive with no thread: threading is a local derivation and
    /// can simply run again.
    pub async fn delete(&self, id: ThreadId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM threads WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    /// Puts a message in a thread and brings the aggregates up to date.
    ///
    /// If the message was in another thread, that one is recomputed too — an
    /// abandoned thread that still claims the message would show a count the
    /// drill-in cannot produce.
    pub async fn add_message(&self, thread_id: ThreadId, message_id: MessageId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            let previous = thread_of(&transaction, message_id).await?;

            // Cached: once per message filed (#728).
            let changed = sql::statement(
                &transaction,
                "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            )
            .await?
            .execute(bind![message_id.get(), thread_id.get()])
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "message",
                    id: message_id.get(),
                });
            }

            recompute_in(&transaction, thread_id).await?;
            if let Some(previous) = previous.filter(|previous| *previous != thread_id) {
                recompute_in(&transaction, previous).await?;
            }
            Ok(())
        })
        .await
    }

    /// Recomputes a thread's aggregates from its members.
    pub async fn recompute(&self, id: ThreadId) -> Result<()> {
        recompute_in(self.connection, id).await
    }

    /// Whether the person took part in conversation `id`: a message of
    /// theirs is in it, filed in a Sent folder (spec 007 FR-111, FR-122).
    ///
    /// One `EXISTS`: a seek on `idx_messages_thread_mailbox` for the
    /// conversation's messages, each one's folder by its key, and it stops at
    /// the first that is Sent.
    pub async fn took_part(&self, id: ThreadId) -> Result<bool> {
        sql::exists(self.connection, Self::explain_took_part(), [id.get()]).await
    }

    /// The SQL [`Self::took_part`] runs.
    pub fn explain_took_part() -> &'static str {
        "SELECT 1 FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
          WHERE m.thread_id = ?1 AND b.role = 'sent' LIMIT 1"
    }

    /// Moves every message from `absorb` into `keep` and deletes `absorb`.
    ///
    /// This is what a late-arriving parent does: two conversations turn out to
    /// have been one all along. Merging into the older thread keeps the id the
    /// UI may already be showing.
    pub async fn merge(&self, keep: ThreadId, absorb: ThreadId) -> Result<()> {
        if keep == absorb {
            return self.recompute(keep).await;
        }
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2",
                bind![keep.get(), absorb.get()],
            )
            .await?;
            // Drafts are shown inline in their thread, so they have to follow.
            sql::execute(
                &transaction,
                "UPDATE drafts SET thread_id = ?1 WHERE thread_id = ?2",
                bind![keep.get(), absorb.get()],
            )
            .await?;
            sql::execute(
                &transaction,
                "DELETE FROM threads WHERE id = ?1",
                [absorb.get()],
            )
            .await?;
            recompute_in(&transaction, keep).await?;
            Ok(())
        })
        .await
    }

    /// The conversation's newest message that a list would show: what a
    /// surfaced reminder's row draws as its first line (spec 007). One
    /// statement, a seek on `idx_messages_thread`.
    pub async fn latest_member(&self, id: ThreadId) -> Result<Option<MessageId>> {
        sql::first(
            self.connection,
            &Self::explain_latest_member(),
            [id.get()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::latest_member`] runs.
    pub fn explain_latest_member() -> String {
        format!(
            "SELECT messages.id FROM messages
              WHERE messages.thread_id = ?1 AND messages.{MEMBER}
              ORDER BY messages.received_at DESC, messages.id DESC LIMIT 1"
        )
    }

    /// Where a row surfaced at `at` goes in Focus's inbox over `inboxes`:
    /// how many of its conversations are newer than `at` (spec 007,
    /// data-model.md "Surfaced rows"). A conversation is newer when any
    /// member there is, since its newest member is the one its row sorts
    /// by. One statement, whatever the inbox holds: a seek on
    /// `idx_messages_list` to the rows newer than `at`.
    ///
    /// With more than one account, a conversation that reached two inboxes
    /// counts in each, where the list folds it into one row, so the row can
    /// sit one place lower than the fold would put it.
    pub async fn focus_position(
        &self,
        inboxes: &[(AccountId, MailboxId)],
        at: DateTime<Utc>,
    ) -> Result<u32> {
        if inboxes.is_empty() {
            return Ok(0);
        }
        let mut arguments: Vec<i64> = inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        arguments.push(to_millis(at));
        let count: i64 = sql::one(
            self.connection,
            &self.explain_focus_position(inboxes.len()),
            arguments,
            |row| row.col(0),
        )
        .await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// The SQL [`Self::focus_position`] runs over `inboxes` inboxes.
    pub fn explain_focus_position(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND {} AND messages.sort_at > ?{}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages."),
            inboxes + 1
        )
    }

    /// A thread's messages as list rows, in either direction.
    pub async fn messages(&self, id: ThreadId, order: ThreadOrder) -> Result<Vec<MessageListRow>> {
        sql::all(
            self.connection,
            &self.explain_messages(order),
            [id.get()],
            read_list_row,
        )
        .await
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

    /// One window of the unified list: every enabled account's inbox, newest
    /// first, conversations grouped across accounts at read time (#184,
    /// ADR 0005 Q2).
    ///
    /// # What is in it
    ///
    /// The **inboxes** (#1692, the maintainer's call of 2026-09-26: "it
    /// should only be inboxes"). A conversation filed away -- archived,
    /// moved, snoozed, deleted -- is not a row here, so a verb that files the
    /// cursor's row takes it out of this view exactly as it does out of a
    /// folder, and `a a a` walks down the list. It used to be every message
    /// of every account, which made triage in Unified change nothing visible.
    ///
    /// Each inbox contributes the rows its own folder list has
    /// ([`ThreadRepository::page`] on [`ThreadListQuery::in_mailbox`]): one
    /// per conversation holding a message there, drawn from its newest
    /// message there, ordered by that message's `(sort_at, id)`. That
    /// order is global -- message ids are unique across accounts -- so the
    /// inboxes' windows merge by it, and one cursor resumes all of them. Each
    /// window is the folder list's own seek over `idx_messages_list`, so a
    /// page reads `limit` rows per inbox whatever the rest of the mail holds.
    ///
    /// # Grouping
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
    /// Only a partner that is itself in view -- in its account's inbox --
    /// counts. A row is emitted at the group's **newest** member by the
    /// list's own order; one with a newer partner in view is that partner's
    /// row, drawn already on this page or an earlier one, so no conversation
    /// is ever two rows. Dedupe is display-only (Q13): `message_count` counts
    /// distinct `RfcMessageId`s across the members, both copies stay, and
    /// [`ThreadGroup::members`] is exactly what an action must expand to.
    pub async fn unified_page(&self, query: &UnifiedThreadListQuery) -> Result<Vec<ThreadGroup>> {
        let inboxes = self.unified_inboxes().await?;
        self.folded_page(Membership::Folder, &inboxes, query.limit, query.after)
            .await
    }

    /// At most `limit` rows of a list over several inboxes after `after`,
    /// each conversation the person received at more than one address
    /// folded into one [`ThreadGroup`]: the grouping
    /// [`Self::unified_page`] describes, for the unified inbox
    /// ([`Membership::Folder`]) and for Focus's ([`Membership::Focus`]),
    /// whose raw window is its own merged statement and whose rows leave
    /// out what Focus holds back.
    ///
    /// A Focus row's participants are left to its caller, which reads them
    /// for the rows it keeps rather than for every raw row read here.
    async fn folded_page(
        &self,
        membership: Membership,
        inboxes: &[(AccountId, MailboxId)],
        limit: u32,
        after: Option<ThreadCursor>,
    ) -> Result<Vec<ThreadGroup>> {
        let mut groups: Vec<ThreadGroup> = Vec::new();
        let mut cursor = after;
        let batch = limit.max(2) * 2;

        // The raw page over-fetches: absorption folds rows together, so a
        // page of rows can under-fill the page of groups. Loop until the
        // groups fill or the list ends; each pass is one seek per inbox.
        'fill: loop {
            let raw = match membership {
                Membership::Folder => self.unified_raw_page(inboxes, batch, cursor).await?,
                Membership::Focus => self.focus_raw_page(inboxes, batch, cursor).await?,
            };
            let Some((_, last)) = raw.last() else {
                break;
            };
            cursor = Some(last.cursor());
            let exhausted = raw.len() < batch as usize;

            let mut partners = self.partners_in_view(membership, inboxes, &raw).await?;
            for (account, row) in raw {
                let partners = row
                    .id
                    .and_then(|id| partners.remove(&id))
                    .unwrap_or_default();
                if partners
                    .iter()
                    .any(|partner| sort_key(partner.at) > sort_key(row.cursor()))
                {
                    continue;
                }
                let members = row
                    .id
                    .map(|thread| (account, thread))
                    .into_iter()
                    .chain(
                        partners
                            .iter()
                            .map(|partner| (partner.thread.account_id, partner.thread.id)),
                    )
                    .collect();
                let row = self.group_row(membership, row, &partners, inboxes).await?;
                groups.push(ThreadGroup { row, members });
                if groups.len() as u32 >= limit {
                    break 'fill;
                }
            }
            if exhausted {
                break;
            }
        }
        Ok(groups)
    }

    /// The inboxes the unified list is made of: each enabled account's own,
    /// as `(account, inbox)`.
    ///
    /// By role, never by name, and one per account -- the first by path when
    /// a server has somehow reported two, the rule
    /// [`MailboxRepository::by_role`](super::MailboxRepository::by_role)
    /// keeps, so the folder a verb routes to and the folder this lists are
    /// the same one. A retired folder (not selectable) is no inbox, and an
    /// account with no inbox yet -- its first sync still to come --
    /// contributes nothing rather than failing the view.
    pub async fn unified_inboxes(&self) -> Result<Vec<(AccountId, MailboxId)>> {
        let rows: Vec<(i64, i64)> = sql::all(
            self.connection,
            &self.explain_unified_inboxes(),
            (),
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let mut inboxes: Vec<(AccountId, MailboxId)> = Vec::new();
        for (account, inbox) in rows {
            let account = AccountId::new(account);
            if inboxes.last().map(|(held, _)| *held) != Some(account) {
                inboxes.push((account, MailboxId::new(inbox)));
            }
        }
        Ok(inboxes)
    }

    /// The SQL [`Self::unified_inboxes`] runs, so a test can ask the planner
    /// about it. Driven from `accounts` -- a handful of rows -- into
    /// `idx_mailboxes_account_role`, so no statement here reads mail.
    pub fn explain_unified_inboxes(&self) -> String {
        "SELECT m.account_id, m.id
           FROM accounts a JOIN mailboxes m
             ON m.account_id = a.id AND m.role = 'inbox'
          WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1
          ORDER BY m.account_id, m.path"
            .to_owned()
    }

    /// One window of the unified list at a row offset, for a list model that
    /// scrolls by index.
    ///
    /// The same bargain [`page_at`](Self::page_at) makes, for the same
    /// reason: `ListWindow` addresses rows by position because "never
    /// materialise a mailbox" requires it, and the grouping walk only knows
    /// how to resume from a cursor. The offset is counted from `after` every
    /// time, so `postio_runtime::store`'s seek marks are what keep it a
    /// handful of rows rather than the whole list.
    ///
    /// It over-fetches by `offset` and drops the head, because absorption
    /// means a group is not a fixed number of rows — the only thing that
    /// knows where the *n*th row starts is the walk that produced the first
    /// *n*.
    pub async fn unified_page_at(
        &self,
        query: &UnifiedThreadListQuery,
        offset: u32,
    ) -> Result<Vec<ThreadGroup>> {
        if offset == 0 {
            return self.unified_page(query).await;
        }
        let mut groups = self
            .unified_page(&UnifiedThreadListQuery {
                limit: query.limit.saturating_add(offset),
                after: query.after,
            })
            .await?;
        if offset as usize >= groups.len() {
            return Ok(Vec::new());
        }
        Ok(groups.split_off(offset as usize))
    }

    /// How many rows the unified list would show.
    ///
    /// The inboxes' own counts -- the same number each folder's list has --
    /// less the rows absorption folds away: a conversation the user received
    /// at two addresses is a row in each inbox and one row here. With one
    /// inbox in view nothing can fold, and the answer is that folder's count.
    ///
    /// With more, the fold is decided by the same rule the page applies --
    /// a row with a newer partner in view is not a row -- over every inbox
    /// conversation, so the count and the walk cannot disagree about what a
    /// row is. That is a pass over the inboxes' keys plus the partner lookup
    /// in batches: bounded by what the inboxes hold, never by the archive,
    /// and `postio_runtime::store` holds the answer until an inbox moves.
    pub async fn unified_count(&self) -> Result<u32> {
        let inboxes = self.unified_inboxes().await?;
        let mut total: i64 = 0;
        for (_, inbox) in &inboxes {
            let count: i64 = sql::one(
                self.connection,
                &folder_count_sql(Membership::Folder),
                [inbox.get()],
                |row| row.col(0),
            )
            .await?;
            total += count;
        }
        if inboxes.len() < 2 {
            return Ok(total.max(0) as u32);
        }
        let absorbed = self.absorbed(Membership::Folder, &inboxes).await?;
        Ok((total - absorbed).max(0) as u32)
    }

    /// How many of `inboxes`' rows fold into another inbox's: the rows the
    /// walk in [`Self::folded_page`] skips because a partner in view is
    /// newer, decided by the same rule over every inbox conversation, so a
    /// count and the walk cannot disagree about what a row is.
    async fn absorbed(
        &self,
        membership: Membership,
        inboxes: &[(AccountId, MailboxId)],
    ) -> Result<i64> {
        // Every inbox conversation's key: its newest message there, the row
        // the folder list draws it from.
        let mut keys: HashMap<ThreadId, ThreadCursor> = HashMap::new();
        for (_, inbox) in inboxes {
            let rows: Vec<(i64, i64, i64)> = sql::all(
                self.connection,
                &format!(
                    "SELECT sort_at, id, thread_id FROM messages
                      WHERE mailbox_id = ?1 AND {} AND thread_id IS NOT NULL
                      ORDER BY sort_at DESC, id DESC",
                    membership.test("")
                ),
                [inbox.get()],
                |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
            )
            .await?;
            for (sort_at, id, thread) in rows {
                keys.entry(ThreadId::new(thread))
                    .or_insert_with(|| ThreadCursor {
                        last_at: from_millis(sort_at),
                        id,
                    });
            }
        }

        let ids: Vec<ThreadId> = keys.keys().copied().collect();
        let mut absorbed: i64 = 0;
        for chunk in ids.chunks(COUNT_BATCH) {
            let threads = self.threads_by_id(chunk).await?;
            let partners = self.group_partners_for(&threads).await?;
            for thread in &threads {
                let own = sort_key(keys[&thread.id]);
                let folded = partners.get(&thread.id).is_some_and(|partners| {
                    partners.iter().any(|partner| {
                        keys.get(&partner.id)
                            .is_some_and(|key| sort_key(*key) > own)
                    })
                });
                if folded {
                    absorbed += 1;
                }
            }
        }
        Ok(absorbed)
    }

    /// One raw window of the inboxes, merged newest first: at most `limit`
    /// rows after `after`, each with the account whose inbox it is in.
    ///
    /// Each inbox is asked for its own `limit` rows after the same cursor --
    /// the folder list's seek -- and the union's first `limit` are exactly the
    /// merged list's, because no inbox can contribute more than it was asked
    /// for to the top `limit` of all of them.
    async fn unified_raw_page(
        &self,
        inboxes: &[(AccountId, MailboxId)],
        limit: u32,
        after: Option<ThreadCursor>,
    ) -> Result<Vec<(AccountId, ThreadListRow)>> {
        let mut merged: Vec<(AccountId, ThreadListRow)> = Vec::new();
        for (account, inbox) in inboxes {
            let query = ThreadListQuery {
                after,
                ..ThreadListQuery::in_mailbox(*account, *inbox).limit(limit)
            };
            merged.extend(
                self.page(&query)
                    .await?
                    .into_iter()
                    .map(|row| (*account, row)),
            );
        }
        merged.sort_by_key(|(_, row)| std::cmp::Reverse(sort_key(row.cursor())));
        merged.truncate(limit as usize);
        Ok(merged)
    }

    /// Threads by id, aggregates only.
    async fn threads_by_id(&self, ids: &[ThreadId]) -> Result<Vec<Thread>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT {THREAD_COLUMNS} FROM threads WHERE id IN ({})",
                placeholders(ids.len(), 1)
            ),
        )
        .await?;
        sql::mapped(
            &mut statement,
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            read_thread,
        )
        .await
    }

    /// Each raw row's partners that are in view -- in their own account's
    /// inbox -- with the key each is drawn at there.
    ///
    /// A partner filed away is not part of what the user is looking at: it
    /// neither takes the row nor folds into it, and an action on the row does
    /// not reach it through the group.
    async fn partners_in_view(
        &self,
        membership: Membership,
        inboxes: &[(AccountId, MailboxId)],
        raw: &[(AccountId, ThreadListRow)],
    ) -> Result<HashMap<ThreadId, Vec<Partner>>> {
        let ids: Vec<ThreadId> = raw.iter().filter_map(|(_, row)| row.id).collect();
        let threads = self.threads_by_id(&ids).await?;
        let candidates = self.group_partners_for(&threads).await?;
        let mut wanted: Vec<ThreadId> = candidates
            .values()
            .flatten()
            .map(|partner| partner.id)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(i64, i64, i64)> = {
            let mut statement = sql::statement(
                self.connection,
                &format!(
                    "SELECT thread_id, sort_at, id FROM messages
                      WHERE thread_id IN ({}) AND mailbox_id IN ({}) AND {}",
                    placeholders(wanted.len(), 1),
                    placeholders(inboxes.len(), wanted.len() + 1),
                    membership.test("")
                ),
            )
            .await?;
            let arguments: Vec<i64> = wanted
                .iter()
                .map(|id| id.get())
                .chain(inboxes.iter().map(|(_, inbox)| inbox.get()))
                .collect();
            sql::mapped(&mut statement, arguments, |row| {
                Ok((row.col(0)?, row.col(1)?, row.col(2)?))
            })
            .await?
        };
        let mut keys: HashMap<ThreadId, ThreadCursor> = HashMap::new();
        for (thread, sort_at, id) in rows {
            let key = ThreadCursor {
                last_at: from_millis(sort_at),
                id,
            };
            keys.entry(ThreadId::new(thread))
                .and_modify(|held| {
                    if sort_key(key) > sort_key(*held) {
                        *held = key;
                    }
                })
                .or_insert(key);
        }

        Ok(candidates
            .into_iter()
            .map(|(thread, partners)| {
                let partners = partners
                    .into_iter()
                    .filter_map(|partner| {
                        keys.get(&partner.id).map(|at| Partner {
                            thread: partner,
                            at: *at,
                        })
                    })
                    .collect();
                (thread, partners)
            })
            .collect())
    }

    /// The threads in *other* accounts that are each page thread's
    /// conversation — by root identity, then by subject within the window.
    ///
    /// Three statements for the whole page, not three per thread: the
    /// per-thread version was the unified page's entire cost.
    async fn group_partners_for(&self, page: &[Thread]) -> Result<HashMap<ThreadId, Vec<Thread>>> {
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
            let mut statement = sql::statement(
                self.connection,
                &format!(
                    "SELECT thread_id, rfc_message_id FROM (
                     SELECT thread_id, rfc_message_id,
                            row_number() OVER (PARTITION BY thread_id
                                               ORDER BY received_at, id) AS rank
                       FROM messages
                      WHERE thread_id IN ({placeholders}) AND {MEMBER}
                 ) WHERE rank = 1"
                ),
            )
            .await?;
            let rows = sql::mapped(&mut statement, ids.clone(), |row| {
                Ok((
                    ThreadId::new(row.col::<i64>(0)?),
                    row.col::<Option<String>>(1)?,
                ))
            })
            .await?;
            for (thread_id, root) in rows {
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
            let mut statement = sql::statement(
                self.connection,
                &format!(
                    "SELECT DISTINCT m.rfc_message_id, {columns} FROM threads t
                   JOIN messages m ON m.thread_id = t.id
                  WHERE m.rfc_message_id IN ({root_placeholders})
                    AND t.message_count > 0 AND {enabled}",
                    columns = prefixed_thread_columns("t"),
                    enabled = enabled_account("t.")
                ),
            )
            .await?;
            let rows = sql::mapped(
                &mut statement,
                root_keys.iter().map(|key| key.as_str()).collect::<Vec<_>>(),
                |row| {
                    let root: String = row.col(0)?;
                    let mut candidate = read_thread_offset(row, 1)?;
                    candidate.message_ids = Vec::new();
                    Ok((root, candidate))
                },
            )
            .await?;
            for (root, candidate) in rows {
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
            let mut statement = sql::statement(
                self.connection,
                &format!(
                    "SELECT {THREAD_COLUMNS} FROM threads
                  WHERE subject IN ({subject_placeholders}) AND message_count > 0
                    AND {enabled}",
                    enabled = enabled_account("")
                ),
            )
            .await?;
            let rows = sql::mapped(&mut statement, subjects.clone(), read_thread).await?;
            for candidate in rows {
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

    /// The group's display row: the head's inbox row, counts deduped across
    /// every member.
    ///
    /// Participants and the representative come with the head's row, read
    /// in one batch per inbox page the way [`ThreadRepository::page`] does.
    async fn group_row(
        &self,
        membership: Membership,
        head: ThreadListRow,
        partners: &[Partner],
        inboxes: &[(AccountId, MailboxId)],
    ) -> Result<ThreadListRow> {
        let mut row = head;
        let Some(head) = row.id else {
            return Ok(row);
        };
        if partners.is_empty() {
            // The overwhelmingly common group: one thread, one account. Its
            // inbox row is already the answer, and asking SQL to dedupe a set
            // of one was most of the unified page's cost.
            return Ok(row);
        }

        // Distinct messages, not distinct rows: a copy received at two
        // addresses is one message to the user. A message with no
        // RfcMessageId can never be anyone's copy, so it counts by row. The
        // badge is the conversation's size wherever it is filed, and the
        // unread count is the inboxes' slice -- a folder row's two rules.
        let mut members: Vec<i64> = vec![head.get()];
        members.extend(partners.iter().map(|partner| partner.thread.id.get()));
        let first_inbox = members.len() + 1;
        let (message_count, unread_count): (u32, u32) = sql::one(
            self.connection,
            &format!(
                "SELECT
                     count(DISTINCT coalesce(nullif(m.rfc_message_id, ''), 'row:' || m.id)),
                     count(DISTINCT CASE WHEN m.seen = 0 AND m.mailbox_id IN ({inboxes}){excludes}
                         THEN coalesce(nullif(m.rfc_message_id, ''), 'row:' || m.id) END)
                   FROM messages m
                  WHERE m.thread_id IN ({members}) AND m.{MEMBER}",
                members = placeholders(members.len(), 1),
                inboxes = placeholders(inboxes.len(), first_inbox),
                excludes = membership.excludes("m."),
            ),
            members
                .iter()
                .copied()
                .chain(inboxes.iter().map(|(_, inbox)| inbox.get()))
                .collect::<Vec<_>>(),
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        row.message_count = message_count;
        row.unread_count = unread_count;
        for partner in partners {
            row.has_attachments |= partner.thread.has_attachments;
            row.is_flagged |= partner.thread.is_flagged;
            row.first_at = row.first_at.min(partner.thread.first_at);
        }
        Ok(row)
    }

    /// One window of Focus's inbox (spec 007): every inbox in the query,
    /// one row per conversation it holds, newest first -- the unified
    /// inbox's membership, asked through [`Membership::Focus`].
    ///
    /// # One account: two statements
    ///
    /// The window is one statement: each inbox's own folder window, sought
    /// through `idx_messages_list` and bounded by the page, merged in the
    /// same statement by `(sort_at, id)` -- the order every inbox's list
    /// already keeps, and a total one, since message ids are unique across
    /// accounts. The merge sorts at most the inboxes times the page, never an
    /// inbox. The representative's list row comes in the same row rather than
    /// by id afterwards, which leaves room for the markers read in the budget
    /// of three (`contracts/engine.md`). Participants are the second.
    ///
    /// # Several: a conversation received at two addresses is one row
    ///
    /// As it is in the unified inbox, and by the same partner search
    /// ([`Self::folded_page`]): a row with a newer partner in another
    /// account's inbox is that partner's row, and [`ThreadGroup::members`]
    /// names both copies, which is what an action on the row must reach.
    /// With one inbox nothing can fold, so the search is not asked and a
    /// one-account page keeps its budget; with more, a page adds what
    /// Unified's does -- five statements for the search, and one to dedupe
    /// each folded row's counts.
    ///
    /// `offset` counts rows from the cursor, for a list model that scrolls
    /// by index, with the caveat [`Self::page_at`] carries: the store's seek
    /// marks are what keep it small.
    pub async fn focus_page_at(
        &self,
        query: &FocusListQuery,
        offset: u32,
    ) -> Result<Vec<ThreadGroup>> {
        if query.inboxes.is_empty() || query.limit == 0 {
            return Ok(Vec::new());
        }
        let mut groups = if query.inboxes.len() < 2 {
            self.focus_window(query, offset)
                .await?
                .into_iter()
                .map(|(account, row)| ThreadGroup {
                    members: row.id.map(|thread| (account, thread)).into_iter().collect(),
                    row,
                })
                .collect()
        } else {
            // A group is not a fixed number of raw rows, so the only thing
            // that knows where the offset-th row starts is the walk that
            // produced the ones before it, as in `unified_page_at`.
            let mut groups = self
                .folded_page(
                    Membership::Focus,
                    &query.inboxes,
                    query.limit.saturating_add(offset),
                    query.after,
                )
                .await?;
            if offset as usize >= groups.len() {
                return Ok(Vec::new());
            }
            groups.split_off(offset as usize)
        };

        let ids: Vec<ThreadId> = groups.iter().filter_map(|group| group.row.id).collect();
        let mut participants = self.participants_for(&ids).await?;
        for ThreadGroup { row, .. } in &mut groups {
            if let Some(id) = row.id {
                row.participants = participants.remove(&id).unwrap_or_default();
            }
            // As the folder's rows do: whoever wrote a conversation of one.
            if row.participants.is_empty()
                && let Some(from) = row.latest.as_ref().and_then(|latest| latest.from.clone())
            {
                row.participants = vec![from];
            }
        }
        self.mark(&query.inboxes, &mut groups).await?;
        Ok(groups)
    }

    /// Gives each row of a Focus page its marker, in one statement for the
    /// page ([`Self::explain_focus_markers`]), on the pattern of
    /// [`Self::participants_for`].
    ///
    /// A folded row draws its own conversation's marker first and a copy's
    /// only when its own has none: the copies are one conversation, so
    /// whichever copy the person's classifier reached first speaks for it.
    async fn mark(
        &self,
        inboxes: &[(AccountId, MailboxId)],
        groups: &mut [ThreadGroup],
    ) -> Result<()> {
        let mut threads: Vec<i64> = groups
            .iter()
            .flat_map(|group| group.members.iter().map(|(_, thread)| thread.get()))
            .collect();
        threads.sort_unstable();
        threads.dedup();
        let lone: Vec<i64> = groups
            .iter()
            .filter(|group| group.row.id.is_none())
            .filter_map(|group| group.row.latest.as_ref().map(|latest| latest.id.get()))
            .collect();
        if threads.is_empty() && lone.is_empty() {
            return Ok(());
        }
        let sql = self.explain_focus_markers(inboxes.len(), threads.len(), lone.len());
        let mut arguments = threads.clone();
        if !threads.is_empty() {
            arguments.extend(inboxes.iter().map(|(_, inbox)| inbox.get()));
        }
        arguments.extend(lone);
        let mut statement = sql::statement(self.connection, &sql).await?;
        let found = sql::mapped(&mut statement, arguments, |row| {
            Ok((
                row.col::<Option<i64>>(0)?,
                row.col::<i64>(1)?,
                read_marker_summary(row, 2)?,
            ))
        })
        .await?;
        drop(statement);

        let mut by_thread: HashMap<ThreadId, MarkerSummary> = HashMap::new();
        let mut by_message: HashMap<MessageId, MarkerSummary> = HashMap::new();
        for (thread, message, marker) in found {
            match thread {
                Some(thread) => by_thread.insert(ThreadId::new(thread), marker),
                None => by_message.insert(MessageId::new(message), marker),
            };
        }
        for ThreadGroup { row, members } in groups {
            row.marker = match row.id {
                Some(_) => members
                    .iter()
                    .find_map(|(_, thread)| by_thread.get(thread).cloned()),
                None => row
                    .latest
                    .as_ref()
                    .and_then(|latest| by_message.get(&latest.id).cloned()),
            };
        }
        Ok(())
    }

    /// Focus's window, `offset` rows in: one statement, each row with the
    /// account whose inbox it is in, and no participants yet.
    async fn focus_window(
        &self,
        query: &FocusListQuery,
        offset: u32,
    ) -> Result<Vec<(AccountId, ThreadListRow)>> {
        let mut arguments: Vec<i64> = query.inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        if let Some(cursor) = query.after {
            arguments.push(to_millis(cursor.last_at));
            arguments.push(cursor.id);
        }
        let mut statement =
            sql::statement(self.connection, &self.explain_focus(query, offset)).await?;
        sql::mapped(&mut statement, arguments, read_focus_row).await
    }

    /// One raw window of Focus's inboxes for [`Self::folded_page`]: at most
    /// `limit` rows after `after`, merged newest first in one statement --
    /// the Focus twin of [`Self::unified_raw_page`], which asks each inbox
    /// in turn.
    async fn focus_raw_page(
        &self,
        inboxes: &[(AccountId, MailboxId)],
        limit: u32,
        after: Option<ThreadCursor>,
    ) -> Result<Vec<(AccountId, ThreadListRow)>> {
        self.focus_window(
            &FocusListQuery {
                inboxes: inboxes.to_vec(),
                limit,
                after,
            },
            0,
        )
        .await
    }

    /// The SQL [`Self::focus_page_at`] runs, for `EXPLAIN QUERY PLAN`: one
    /// inbox's window alone, or several merged. The query names at least one
    /// inbox.
    pub fn explain_focus(&self, query: &FocusListQuery, offset: u32) -> String {
        let inboxes = query.inboxes.len();
        let cursor = query.after.map(|_| (inboxes + 1, inboxes + 2));
        let skip = if offset > 0 {
            format!(" OFFSET {offset}")
        } else {
            String::new()
        };
        if inboxes == 1 {
            return format!("{}{skip}", focus_arm(1, cursor, query.limit));
        }
        // Each inbox is asked for as many rows as the page could need from
        // it, and the merge keeps the page's.
        let each = query.limit.saturating_add(offset);
        let arms = (1..=inboxes)
            .map(|mailbox| format!("SELECT * FROM ({})", focus_arm(mailbox, cursor, each)))
            .collect::<Vec<_>>()
            .join("\n UNION ALL\n");
        format!(
            "{arms}\n ORDER BY focus_at DESC, focus_id DESC LIMIT {}{skip}",
            query.limit
        )
    }

    /// The SQL a Focus page reads its markers with, for `inboxes` inboxes,
    /// `threads` conversations and `lone` messages in none: for each
    /// conversation, its newest message in the inboxes' Focus slice that
    /// carries a marker the person has not dismissed -- one row per
    /// conversation that draws a marker, and none for one that does not.
    ///
    /// Driven from the conversations' own index, each bounded by the size of
    /// one conversation, and into `markers` by its key: never a walk of
    /// either table. `m.mailbox_id + 0` is what keeps it there -- with the
    /// column bare the planner seeks the inbox instead and walks all of it,
    /// a cost of the inbox rather than of the page. The newest marked
    /// message is `max()`'s row, whose bare columns the engine answers from,
    /// the pattern [`Self::participants_for`] relies on with `min()`.
    pub fn explain_focus_markers(&self, inboxes: usize, threads: usize, lone: usize) -> String {
        let columns = "k.kind, k.starts_at, k.ends_at, k.due_at, k.excerpt, k.answer, \
                       k.invite_state";
        let mut arms = Vec::new();
        if threads > 0 {
            arms.push(format!(
                "SELECT m.thread_id, m.id, {columns}, max(m.received_at)
                   FROM messages m JOIN markers k ON k.message_id = m.id
                  WHERE m.thread_id IN ({threads}) AND m.mailbox_id + 0 IN ({inboxes})
                    AND {member} AND k.dismissed_at IS NULL
                  GROUP BY m.thread_id",
                threads = placeholders(threads, 1),
                inboxes = placeholders(inboxes, threads + 1),
                member = Membership::Focus.test("m."),
            ));
        }
        if lone > 0 {
            arms.push(format!(
                "SELECT NULL, m.id, {columns}, m.received_at
                   FROM messages m JOIN markers k ON k.message_id = m.id
                  WHERE m.id IN ({lone}) AND k.dismissed_at IS NULL",
                lone = placeholders(lone, threads + inboxes + 1),
            ));
        }
        arms.join("\n UNION ALL\n")
    }

    /// How many rows Focus's inbox has over `inboxes`: one per conversation
    /// each holds, plus one per message there in none, less the rows that
    /// fold into another inbox's -- the rows [`Self::focus_page_at`] walks.
    ///
    /// The first part is one statement, from `idx_messages_mailbox_threads`
    /// alone: a conversation belongs to one account and each account gives
    /// the list one inbox, so the distinct keys across every inbox are the
    /// sum of each inbox's own. With one inbox nothing can fold and that is
    /// the answer. With more, the fold is counted as the unified inbox's is
    /// ([`Self::unified_count`]), over the same membership as the rows.
    pub async fn focus_count(&self, inboxes: &[(AccountId, MailboxId)]) -> Result<u32> {
        if inboxes.is_empty() {
            return Ok(0);
        }
        let count: i64 = sql::one(
            self.connection,
            &self.explain_focus_count(inboxes.len()),
            inboxes
                .iter()
                .map(|(_, inbox)| inbox.get())
                .collect::<Vec<_>>(),
            |row| row.col(0),
        )
        .await?;
        if inboxes.len() < 2 {
            return Ok(count.max(0) as u32);
        }
        let absorbed = self.absorbed(Membership::Focus, inboxes).await?;
        Ok((count - absorbed).max(0) as u32)
    }

    /// The SQL [`Self::focus_count`] runs over `inboxes` inboxes.
    pub fn explain_focus_count(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages.")
        )
    }

    /// Which conversations of Focus's inbox over `inboxes` draw a marker:
    /// the has-action filter's rows (spec 007 US1 scenario 5, T048).
    ///
    /// One statement, driven from the markers the person has not dismissed
    /// -- a few hundred at most, where an inbox is a hundred thousand --
    /// through `idx_markers_dismissed`, and from each to its message by key.
    /// A marker counts where the row would draw it: on a message in the
    /// inboxes' Focus slice, so an archived, snoozed or held message's
    /// marker makes no row. `idx_markers_open` was planned as a partial
    /// index over open markers, which this engine's planner does not read.
    pub async fn focus_marked(&self, inboxes: &[(AccountId, MailboxId)]) -> Result<Marked> {
        if inboxes.is_empty() {
            return Ok(Marked::default());
        }
        let arguments: Vec<i64> = inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        let found: Vec<(Option<i64>, i64)> = sql::all(
            self.connection,
            &self.explain_focus_marked(inboxes.len()),
            arguments,
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let mut marked = Marked::default();
        for (thread, message) in found {
            match thread {
                Some(thread) => marked.threads.push(ThreadId::new(thread)),
                None => marked.lone.push(MessageId::new(message)),
            }
        }
        Ok(marked)
    }

    /// The SQL [`Self::focus_marked`] runs over `inboxes` inboxes.
    pub fn explain_focus_marked(&self, inboxes: usize) -> String {
        format!(
            "SELECT DISTINCT m.thread_id, CASE WHEN m.thread_id IS NULL THEN m.id ELSE 0 END
               FROM markers k JOIN messages m ON m.id = k.message_id
              WHERE k.dismissed_at IS NULL AND m.mailbox_id IN ({}) AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("m.")
        )
    }

    /// One page of the has-action filter: the rows of `marked`, drawn as
    /// Focus's inbox draws them -- each conversation from its newest message
    /// in the inbox, with its participants and its marker -- newest first,
    /// `offset` rows in.
    ///
    /// Three statements, the inbox page's budget: the window, sought through
    /// the marked conversations' own index and sorting only what they hold;
    /// the participants; and the markers. A conversation received at two
    /// addresses is not folded here: the filter is short, and every copy
    /// that draws a marker is a row.
    pub async fn focus_marked_page(
        &self,
        inboxes: &[(AccountId, MailboxId)],
        marked: &Marked,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ThreadGroup>> {
        if inboxes.is_empty() || marked.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut arguments: Vec<i64> = inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        arguments.extend(marked.threads.iter().map(|thread| thread.get()));
        arguments.extend(marked.lone.iter().map(|message| message.get()));
        let sql = self.explain_focus_marked_page(
            inboxes.len(),
            marked.threads.len(),
            marked.lone.len(),
            offset,
            limit,
        );
        let mut statement = sql::statement(self.connection, &sql).await?;
        let found = sql::mapped(&mut statement, arguments, read_focus_row).await?;
        drop(statement);
        let mut groups: Vec<ThreadGroup> = found
            .into_iter()
            .map(|(account, row)| ThreadGroup {
                members: row.id.map(|thread| (account, thread)).into_iter().collect(),
                row,
            })
            .collect();

        let ids: Vec<ThreadId> = groups.iter().filter_map(|group| group.row.id).collect();
        let mut participants = self.participants_for(&ids).await?;
        for ThreadGroup { row, .. } in &mut groups {
            if let Some(id) = row.id {
                row.participants = participants.remove(&id).unwrap_or_default();
            }
            if row.participants.is_empty()
                && let Some(from) = row.latest.as_ref().and_then(|latest| latest.from.clone())
            {
                row.participants = vec![from];
            }
        }
        self.mark(inboxes, &mut groups).await?;
        Ok(groups)
    }

    /// The SQL [`Self::focus_marked_page`] runs for `inboxes` inboxes,
    /// `threads` marked conversations and `lone` marked messages in none.
    pub fn explain_focus_marked_page(
        &self,
        inboxes: usize,
        threads: usize,
        lone: usize,
        offset: u32,
        limit: u32,
    ) -> String {
        let thread_list = placeholders(threads, inboxes + 1);
        let lone_list = placeholders(lone, inboxes + threads + 1);
        let chosen = match (threads > 0, lone > 0) {
            (true, true) => format!(
                "(rep.thread_id IN ({thread_list}) \
                 OR (rep.thread_id IS NULL AND rep.id IN ({lone_list})))"
            ),
            (true, false) => format!("rep.thread_id IN ({thread_list})"),
            (false, true) => format!("(rep.thread_id IS NULL AND rep.id IN ({lone_list}))"),
            (false, false) => "0".to_owned(),
        };
        let member = |alias: &str| Membership::Focus.test(alias);
        let slice = format!(
            "FROM messages m
              WHERE m.thread_id = rep.thread_id AND m.mailbox_id = rep.mailbox_id AND {}",
            member("m.")
        );
        let skip = if offset > 0 {
            format!(" OFFSET {offset}")
        } else {
            String::new()
        };
        format!(
            "SELECT coalesce(rep.thread_id, 0), rep.account_id, rep.subject,
                    coalesce((SELECT t.message_count FROM threads t
                               WHERE t.id = rep.thread_id), 1),
                    coalesce((SELECT count(*) {slice} AND m.seen = 0),
                             CASE WHEN rep.seen = 0 THEN 1 ELSE 0 END),
                    coalesce((SELECT max(m.has_attachments) {slice}), rep.has_attachments),
                    coalesce((SELECT max(m.flagged) {slice}), rep.flagged),
                    rep.received_at, rep.sort_at AS focus_at, rep.id AS focus_id,
                    {representative}
               FROM messages rep
              WHERE rep.mailbox_id IN ({inbox_list}) AND {rep_member} AND {chosen}
                AND NOT EXISTS (
                        SELECT 1 FROM messages newer
                         WHERE newer.mailbox_id = rep.mailbox_id AND {newer_member}
                           AND newer.thread_id IS NOT NULL
                           AND newer.thread_id = rep.thread_id
                           AND (newer.sort_at, newer.id) > (rep.sort_at, rep.id)
                    )
              ORDER BY rep.sort_at DESC, rep.id DESC LIMIT {limit}{skip}",
            representative = list_columns_of("rep"),
            inbox_list = placeholders(inboxes, 1),
            rep_member = member("rep."),
            newer_member = member("newer."),
        )
    }

    /// How many of Focus's rows over `inboxes` are unread: conversations
    /// with an unread message in the inboxes' Focus slice, and unread
    /// messages in none. One statement, sought through each inbox's index.
    ///
    /// With several accounts, a conversation received unread at two
    /// addresses counts once an inbox, where its row is folded into one:
    /// the fold's partner search is the page's, and a header count does not
    /// pay for it.
    pub async fn focus_unread(&self, inboxes: &[(AccountId, MailboxId)]) -> Result<u32> {
        if inboxes.is_empty() {
            return Ok(0);
        }
        let count: i64 = sql::one(
            self.connection,
            &self.explain_focus_unread(inboxes.len()),
            inboxes
                .iter()
                .map(|(_, inbox)| inbox.get())
                .collect::<Vec<_>>(),
            |row| row.col(0),
        )
        .await?;
        Ok(count.max(0) as u32)
    }

    /// The SQL [`Self::focus_unread`] runs over `inboxes` inboxes.
    pub fn explain_focus_unread(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND messages.seen = 0 AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages.")
        )
    }

    /// One window of the thread list, most recently active first.
    pub async fn page(&self, query: &ThreadListQuery) -> Result<Vec<ThreadListRow>> {
        self.page_with(query, "").await
    }

    /// [`ThreadRepository::page`] with `tail` appended to the statement.
    async fn page_with(&self, query: &ThreadListQuery, tail: &str) -> Result<Vec<ThreadListRow>> {
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
        let sql = format!("{}{tail}", self.explain(query));
        self.rows_from(&sql, arguments, query.mailbox.is_some())
            .await
    }

    /// The rows the list shows for the conversations these messages belong
    /// to, in the page's own shape -- one row per touched conversation,
    /// aggregates, representative and participants included -- and nothing
    /// else (#1607).
    ///
    /// A flag change names message ids, and the list used to re-read a whole
    /// fifty-row page, four correlated subqueries per row, to learn one
    /// conversation's new unread count. Two statements here before the
    /// page's own two: which conversations the ids touch (a point read per
    /// id), then the representatives of exactly those conversations, sought
    /// through the thread index rather than walked through the folder. A
    /// message with no conversation is its own row, as it is on the page.
    /// In an account-wide list only conversations are rows, so a lone
    /// message contributes nothing there.
    pub async fn rows_for(
        &self,
        query: &ThreadListQuery,
        messages: &[MessageId],
    ) -> Result<Vec<ThreadListRow>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT id, thread_id FROM messages WHERE id IN ({})",
                placeholders(messages.len(), 1)
            ),
        )
        .await?;
        let ids: Vec<i64> = messages.iter().map(|id| id.get()).collect();
        let touched: Vec<(i64, Option<i64>)> =
            sql::mapped(&mut statement, ids, |row| Ok((row.col(0)?, row.col(1)?))).await?;
        drop(statement);

        let scoped = query.mailbox.is_some();
        let mut threads: Vec<i64> = touched.iter().filter_map(|(_, thread)| *thread).collect();
        threads.sort_unstable();
        threads.dedup();
        let lone: Vec<i64> = if scoped {
            touched
                .iter()
                .filter(|(_, thread)| thread.is_none())
                .map(|(id, _)| *id)
                .collect()
        } else {
            Vec::new()
        };
        if threads.is_empty() && lone.is_empty() {
            return Ok(Vec::new());
        }

        let sql = self.explain_rows_for(query, threads.len(), lone.len());
        let mut arguments = vec![query.account_id.get()];
        if let Some(mailbox) = query.mailbox {
            arguments.push(mailbox.get());
        }
        arguments.extend(threads);
        arguments.extend(lone);
        self.rows_from(&sql, arguments, scoped).await
    }

    /// How many rows a folder lost when these messages left it (#1607).
    ///
    /// An archive moves `mailboxes.total_count`, which is the folder count's
    /// witness, so the next page used to pay the whole conversation count
    /// again: 786 ms on a real folder, in front of the first row. What the
    /// count lost is knowable from the removed ids alone -- the conversations
    /// none of whose members are still in the folder, plus every lone
    /// message, each of which was its own row -- in two statements bounded
    /// by the ids, not the folder. The removed rows still exist, elsewhere
    /// or hidden pending a remote delete, so their conversations can be read
    /// off them.
    pub async fn conversations_gone_from(
        &self,
        mailbox: MailboxId,
        removed: &[MessageId],
    ) -> Result<u32> {
        if removed.is_empty() {
            return Ok(0);
        }
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT thread_id FROM messages WHERE id IN ({})",
                placeholders(removed.len(), 1)
            ),
        )
        .await?;
        let ids: Vec<i64> = removed.iter().map(|id| id.get()).collect();
        let threads_of: Vec<Option<i64>> =
            sql::mapped(&mut statement, ids, |row| row.col(0)).await?;
        drop(statement);
        let lone = threads_of.iter().filter(|thread| thread.is_none()).count() as u32;
        let mut threads: Vec<i64> = threads_of.into_iter().flatten().collect();
        threads.sort_unstable();
        threads.dedup();
        if threads.is_empty() {
            return Ok(lone);
        }
        let sql = format!(
            "SELECT count(*) FROM threads t
              WHERE t.id IN ({})
                AND NOT EXISTS (SELECT 1 FROM messages m
                                 WHERE m.thread_id = t.id AND m.mailbox_id = ?1 AND {})",
            placeholders(threads.len(), 2),
            Membership::Folder.test("m.")
        );
        let mut arguments = vec![mailbox.get()];
        arguments.extend(threads);
        let emptied: i64 = sql::one(self.connection, &sql, arguments, |row| row.col(0)).await?;
        Ok(lone + emptied as u32)
    }

    /// The SQL [`Self::rows_for`] reads representatives with, for `threads`
    /// conversation ids and `lone` message ids, so a test can ask the
    /// planner about it the way [`Self::explain`] lets it ask about a page.
    pub fn explain_rows_for(&self, query: &ThreadListQuery, threads: usize, lone: usize) -> String {
        let Some(_) = query.mailbox else {
            return format!(
                "SELECT {THREAD_COLUMNS} FROM threads
                  WHERE account_id = ?1 AND message_count > 0
                    AND id IN ({})",
                placeholders(threads, 2)
            );
        };
        let slice = slice(Membership::Folder, "?2");
        let representatives = |filter: String| {
            let window = representative_filter(Membership::Folder, "?2", &format!(" AND {filter}"));
            format!(
                "SELECT coalesce(rep.thread_id, 0), ?1, rep.subject,
                        coalesce((SELECT t.message_count FROM threads t
                                   WHERE t.id = rep.thread_id), 1),
                        coalesce((SELECT count(*) {slice} AND m.seen = 0),
                                 CASE WHEN rep.seen = 0 THEN 1 ELSE 0 END),
                        coalesce((SELECT max(m.has_attachments) {slice}), rep.has_attachments),
                        coalesce((SELECT max(m.flagged) {slice}), rep.flagged),
                        rep.received_at, rep.sort_at, rep.id
                   FROM messages rep
                  WHERE {window}"
            )
        };
        let mut arms = Vec::new();
        if threads > 0 {
            arms.push(representatives(format!(
                "rep.thread_id IN ({})",
                placeholders(threads, 3)
            )));
        }
        if lone > 0 {
            arms.push(representatives(format!(
                "rep.thread_id IS NULL AND rep.id IN ({})",
                placeholders(lone, 3 + threads)
            )));
        }
        arms.join("\n UNION ALL\n")
    }

    /// One conversation row per result of `sql`, filled out the way a page
    /// is: participants, and for a folder-scoped read the representative
    /// message. `arguments` are the statement's, in placeholder order.
    async fn rows_from(
        &self,
        sql: &str,
        arguments: Vec<i64>,
        scoped: bool,
    ) -> Result<Vec<ThreadListRow>> {
        let mut statement = sql::statement(self.connection, sql).await?;
        let rows = sql::mapped(&mut statement, arguments, |row| {
            let thread = row.col::<i64>(0)?;
            Ok((
                ThreadListRow {
                    // Zero is the folder window's spelling of "no thread": an
                    // id column cannot be null and still be compared, so the
                    // query coalesces and this un-coalesces.
                    id: (thread != 0).then(|| ThreadId::new(thread)),
                    subject: row.col(2)?,
                    participants: Vec::new(),
                    message_count: row.col(3)?,
                    unread_count: row.col(4)?,
                    has_attachments: row.col(5)?,
                    is_flagged: row.col(6)?,
                    first_at: from_millis(row.col(7)?),
                    last_at: from_millis(row.col(8)?),
                    latest: None,
                    sort_id: if scoped { row.col(9)? } else { thread },
                    marker: None,
                },
                // The representative's id, which the folder window already
                // knows and the account window has to look up.
                scoped.then(|| MessageId::new(row.col::<i64>(9).unwrap_or_default())),
            ))
        })
        .await?;
        let mut page: Vec<(ThreadListRow, Option<MessageId>)> = rows;
        drop(statement);

        // Two more statements for the whole page, rather than two per row.
        let ids: Vec<ThreadId> = page.iter().filter_map(|(row, _)| row.id).collect();
        let mut participants = self.participants_for(&ids).await?;
        if scoped {
            // The window already named the representative of every row, so
            // this is one read by id rather than a window function over the
            // conversations.
            let wanted: Vec<MessageId> = page.iter().filter_map(|(_, id)| *id).collect();
            let mut latest: HashMap<MessageId, MessageListRow> =
                MessageRepository::new(self.connection)
                    .rows_for(&wanted)
                    .await?
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
            let mut latest = self.latest_messages_for(&ids, None).await?;
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
    /// `idx_messages_thread_mailbox (thread_id, mailbox_id, sort_at DESC,
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
                " AND last_at <= ?2 AND (last_at < ?2 OR id < ?3)"
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
            " AND rep.sort_at <= ?3 AND (rep.sort_at < ?3 OR rep.id < ?4)"
        } else {
            ""
        };
        // The folder's slice of this row's conversation. Spelled once and
        // reused, so the aggregates cannot drift apart on what counts as a
        // member here.
        let slice = slice(Membership::Folder, "?2");
        let window = representative_filter(Membership::Folder, "?2", "");
        format!(
            "SELECT coalesce(rep.thread_id, 0), ?1, rep.subject,
                    coalesce((SELECT t.message_count FROM threads t
                               WHERE t.id = rep.thread_id), 1),
                    coalesce((SELECT count(*) {slice} AND m.seen = 0),
                             CASE WHEN rep.seen = 0 THEN 1 ELSE 0 END),
                    coalesce((SELECT max(m.has_attachments) {slice}), rep.has_attachments),
                    coalesce((SELECT max(m.flagged) {slice}), rep.flagged),
                    rep.received_at, rep.sort_at, rep.id
               FROM messages rep
              WHERE {window}{cursor}
              ORDER BY rep.sort_at DESC, rep.id DESC LIMIT {}",
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
    pub async fn page_at(
        &self,
        query: &ThreadListQuery,
        offset: u32,
    ) -> Result<Vec<ThreadListRow>> {
        if offset == 0 {
            return self.page(query).await;
        }
        self.page_with(query, &format!(" OFFSET {offset}")).await
    }

    /// Where every `stride`-th row of a folder's list begins, as the cursor a
    /// page resumes after: `(offset, cursor)` for offsets `stride`,
    /// `2 * stride`, ... -- what a scrollbar jump seeks from instead of
    /// skipping (#1610).
    pub async fn boundaries(
        &self,
        query: &ThreadListQuery,
        stride: u32,
    ) -> Result<Vec<(u32, ThreadCursor)>> {
        let Some(mailbox) = query.mailbox else {
            return Ok(Vec::new());
        };
        let stride = stride.max(1);
        // One pass over the folder's own index, newest first, keys only --
        // no correlated predicate per row. A row of the list is the first
        // message of its conversation met in that order (the representative
        // the window's `NOT EXISTS` asks for), or a message in no
        // conversation at all; counting them as they go by is counting the
        // list's rows.
        let mut rows = sql::statement(
            self.connection,
            &format!(
                "SELECT sort_at, id, thread_id FROM messages
                  WHERE mailbox_id = ?1 AND {}
                  ORDER BY sort_at DESC, id DESC",
                Membership::Folder.test("")
            ),
        )
        .await?
        .query([mailbox.get()])
        .await?;
        let mut seen = std::collections::HashSet::new();
        let mut counted = 0u32;
        let mut boundaries = Vec::new();
        while let Some(row) = rows.next().await? {
            let thread: Option<i64> = row.col(2)?;
            if thread.is_some_and(|thread| !seen.insert(thread)) {
                continue;
            }
            counted += 1;
            if counted.is_multiple_of(stride) {
                boundaries.push((
                    counted,
                    ThreadCursor {
                        last_at: from_millis(row.col(0)?),
                        id: row.col(1)?,
                    },
                ));
            }
        }
        // A boundary at the very end begins no page.
        if boundaries
            .last()
            .is_some_and(|(offset, _)| *offset == counted)
        {
            boundaries.pop();
        }
        Ok(boundaries)
    }

    /// How many threads the list would show.
    pub async fn count(&self, account_id: AccountId) -> Result<u32> {
        self.count_of(&ThreadListQuery::account(account_id)).await
    }

    /// How many threads `query`'s scope would show.
    ///
    /// The folder-scoped count is the same `EXISTS` the page uses, so the
    /// number and the rows cannot disagree about what "in this folder" means.
    pub async fn count_of(&self, query: &ThreadListQuery) -> Result<u32> {
        let count: i64 = match query.mailbox {
            None => {
                sql::one(
                    self.connection,
                    "SELECT count(*) FROM threads WHERE account_id = ?1 AND message_count > 0",
                    [query.account_id.get()],
                    |row| row.col(0),
                )
                .await?
            }
            // The same predicate the window uses, so the number and the rows
            // cannot disagree about what a row is: one per conversation the
            // folder holds, plus one per message it holds that belongs to no
            // conversation.
            Some(mailbox) => {
                sql::one(
                    self.connection,
                    &self.explain_count_of(),
                    [mailbox.get()],
                    |row| row.col(0),
                )
                .await?
            }
        };
        Ok(count as u32)
    }

    /// The SQL [`Self::count_of`] counts a folder with, so a test can ask the
    /// planner about it.
    pub fn explain_count_of(&self) -> String {
        folder_count_sql(Membership::Folder)
    }

    /// The members of a thread, oldest first.
    pub async fn member_ids(&self, id: ThreadId) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT id FROM messages WHERE thread_id = ?1 AND {MEMBER}
              ORDER BY received_at, id"
            ),
            [id.get()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// Everyone who has written in each of `ids`, in first-seen order.
    ///
    /// One statement for the whole page. `min(received_at)` is what orders
    /// them, and SQLite takes the bare `name`/`address` columns from the row
    /// that minimum came from — so the display name is the one the participant
    /// first appeared under.
    async fn participants_for(
        &self,
        ids: &[ThreadId],
    ) -> Result<HashMap<ThreadId, Vec<EmailAddress>>> {
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
        let mut statement = sql::statement(self.connection, &sql).await?;
        let rows = sql::mapped(
            &mut statement,
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            |row| {
                Ok((
                    ThreadId::new(row.col(0)?),
                    EmailAddress::new(row.col::<Option<String>>(1)?, row.col::<String>(2)?),
                ))
            },
        )
        .await?;

        let mut participants: HashMap<ThreadId, Vec<EmailAddress>> = HashMap::new();
        for (thread_id, address) in rows {
            participants.entry(thread_id).or_default().push(address);
        }
        Ok(participants)
    }

    /// The newest message in each of `ids`, as a list row.
    ///
    /// One statement for the whole page: `row_number()` picks the newest per
    /// thread, and the sender lookups then run only for the rows that survive.
    async fn latest_messages_for(
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
                                           ORDER BY sort_at DESC, id DESC) AS rank
                   FROM messages
                  WHERE thread_id IN ({}) AND {MEMBER}{scope}
             )
             SELECT {LIST_COLUMNS}
               FROM ranked JOIN messages ON messages.id = ranked.id
              WHERE ranked.rank = 1",
            placeholders(ids.len(), 1)
        );
        let mut statement = sql::statement(self.connection, &sql).await?;
        let rows = sql::mapped(
            &mut statement,
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            |row| {
                // `LIST_COLUMNS` rather than a hand-written copy of it. This
                // query used to spell the same thirteen columns out again, and
                // adding a fourteenth to `LIST_COLUMNS` left this one behind --
                // `read_list_row` then asked for a column index the statement did
                // not have, and the unified list failed to page at all while every
                // storage-level test passed. `thread_id` is the second column in
                // both, which is what lets the reader be shared.
                Ok((ThreadId::new(row.col(1)?), read_list_row(row)?))
            },
        )
        .await?;

        let mut latest = HashMap::new();
        for (thread_id, message) in rows {
            latest.insert(thread_id, message);
        }
        Ok(latest)
    }

    async fn mailboxes_in(&self, id: ThreadId) -> Result<Vec<MailboxId>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT DISTINCT mailbox_id FROM messages WHERE thread_id = ?1 AND {MEMBER}
              ORDER BY mailbox_id"
            ),
            [id.get()],
            |row| Ok(MailboxId::new(row.col(0)?)),
        )
        .await
    }

    async fn labels_in(&self, id: ThreadId) -> Result<Vec<LabelId>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT DISTINCT message_labels.label_id
               FROM message_labels
               JOIN messages ON messages.id = message_labels.message_id
              WHERE messages.thread_id = ?1 AND messages.{MEMBER}
              ORDER BY message_labels.label_id"
            ),
            [id.get()],
            |row| Ok(LabelId::new(row.col(0)?)),
        )
        .await
    }
}

/// The thread a message is currently in.
async fn thread_of(connection: &Connection, message_id: MessageId) -> Result<Option<ThreadId>> {
    let mut statement =
        sql::statement(connection, "SELECT thread_id FROM messages WHERE id = ?1").await?;
    Ok(
        crate::sql::first_of(&mut statement, [message_id.get()], |row| {
            row.col::<Option<i64>>(0)
        })
        .await?
        .flatten()
        .map(ThreadId::new),
    )
}

/// Recomputes one thread's aggregates from its members, in whatever
/// transaction the caller is already in.
///
/// The subject is the normalized subject of the oldest member — the message
/// that named the conversation — recomputed here because a merge can change
/// which message that is.
async fn recompute_in(connection: &Connection, id: ThreadId) -> Result<()> {
    let root_subject: Option<String> = sql::one(
        connection,
        &format!(
            "SELECT subject FROM messages WHERE thread_id = ?1 AND {MEMBER}
                  ORDER BY received_at, id LIMIT 1"
        ),
        [id.get()],
        |row| row.col::<Option<String>>(0),
    )
    .await
    .unwrap_or(None);

    sql::execute(
        connection,
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
                    last_at = coalesce((SELECT max(sort_at) FROM messages
                                         WHERE thread_id = ?1 AND {MEMBER}), 0)
              WHERE id = ?1"
        ),
        bind![id.get(), root_subject.as_deref().map(normalize_subject)],
    )
    .await?;
    Ok(())
}

/// What a row draws of a marker, from the columns
/// [`ThreadRepository::explain_focus_markers`] reads starting at `first`:
/// kind, event start and end, due date, excerpt, answer, invitation state.
fn read_marker_summary(row: &Row, first: usize) -> Result<MarkerSummary> {
    let time = |index: usize| -> Result<Option<DateTime<Utc>>> {
        Ok(row.col::<Option<i64>>(first + index)?.map(from_millis))
    };
    let when = match (time(1)?, time(2)?, time(3)?) {
        (Some(starts_at), Some(ends_at), _) => Some(MarkerWhen::Event { starts_at, ends_at }),
        (_, _, Some(due)) => Some(MarkerWhen::Due(due)),
        _ => None,
    };
    let state = row
        .col::<Option<String>>(first + 6)?
        .as_deref()
        .map(super::markers::state_of)
        .transpose()?;
    Ok(MarkerSummary {
        kind: super::markers::kind_of(&row.col::<String>(first)?)?,
        when,
        excerpt: row.col(first + 4)?,
        answer: row
            .col::<Option<String>>(first + 5)?
            .as_deref()
            .map(super::markers::answer_of)
            .transpose()?,
        cancelled: state == Some(super::InviteState::Cancelled),
    })
}

/// [`read_thread`], with the thread's columns starting at `offset`.
fn read_thread_offset(row: &Row, offset: usize) -> Result<Thread> {
    Ok(Thread {
        id: ThreadId::new(row.col(offset)?),
        account_id: AccountId::new(row.col(offset + 1)?),
        subject: row.col(offset + 2)?,
        message_ids: Vec::new(),
        participants: Vec::new(),
        mailbox_ids: Vec::new(),
        labels: Vec::new(),
        message_count: row.col(offset + 3)?,
        unread_count: row.col(offset + 4)?,
        has_attachments: row.col(offset + 5)?,
        is_flagged: row.col(offset + 6)?,
        first_at: from_millis(row.col(offset + 7)?),
        last_at: from_millis(row.col(offset + 8)?),
    })
}

fn read_thread(row: &Row) -> Result<Thread> {
    Ok(Thread {
        id: ThreadId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        subject: row.col(2)?,
        message_ids: Vec::new(),
        participants: Vec::new(),
        mailbox_ids: Vec::new(),
        labels: Vec::new(),
        message_count: row.col(3)?,
        unread_count: row.col(4)?,
        has_attachments: row.col(5)?,
        is_flagged: row.col(6)?,
        first_at: from_millis(row.col(7)?),
        last_at: from_millis(row.col(8)?),
    })
}
