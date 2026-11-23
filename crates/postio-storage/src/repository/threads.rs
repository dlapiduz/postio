//! Threads: the conversation a message belongs to, and the row the list shows. `threads` holds only
//! aggregates, recomputed by [`ThreadRepository::recompute`] after every mutation; membership is
//! `messages.thread_id` alone, with everything else derived, so there is one place to be wrong.
//! No N+1: [`ThreadRepository::page`] fetches the page, its participants and its newest messages in
//! three statements; `tests/threads.rs` counts them.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen};
use postio_model::mime::preview;
use postio_model::thread_view::before_attribution;
use postio_model::{
    AccountId, DraftId, EmailAddress, LabelId, Line, MailboxId, MessageId, RfcMessageId, Thread,
    ThreadDraft, ThreadId, ThreadMember, ThreadNode, descendants, normalize_subject,
};

use super::messages::{
    LIST_COLUMNS, MessageListRow, list_columns_of, placeholders, read_list_row, read_list_row_at,
};
use super::{from_millis, require_persisted, to_millis};

use crate::error::{Error, Result};
use crate::repository::MessageRepository;
use crate::sql::{self, Bind as _, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

pub const DEFAULT_THREAD_PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadOrder {
    Oldest,
    Newest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadCursor {
    pub last_at: DateTime<Utc>,
    /// Which id depends on the list: the thread's in an account-scoped list, the representative
    /// message's in a folder-scoped one; only compared against the column it came from.
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListQuery {
    pub account_id: AccountId,
    /// A real folder threads (ADR 0015): only conversations holding a message here appear, the row
    /// is drawn from the newest message here, and the unread count is this folder's slice. `None`
    /// is the whole account. The total message count is never scoped.
    pub mailbox: Option<MailboxId>,
    pub limit: u32,
    pub after: Option<ThreadCursor>,
}

#[derive(Debug, Clone, Copy)]
pub struct UnifiedThreadListQuery {
    pub limit: u32,
    pub after: Option<ThreadCursor>,
}

/// `row` is what the list draws; `members` is what an action expands to: one thread per account
/// holding a copy, so "archive" is two operations in two per-account queues (ADR 0005 Q2). See
/// [`ThreadRepository::unified_page`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadGroup {
    pub row: ThreadListRow,
    pub members: Vec<(AccountId, ThreadId)>,
}

impl ThreadGroup {
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            last_at: self.row.last_at,
            id: self.row.sort_id,
        }
    }
}

/// A disabled account drops out silently (ADR 0005 Q10). Applied to the partner search as well as
/// the page: a disabled thread that absorbed its enabled partner would fold a visible row into an
/// invisible one.
fn enabled_account(alias: &str) -> String {
    format!(
        "{alias}account_id IN \
         (SELECT id FROM accounts WHERE enabled = 1 AND pending_deletion = 0)"
    )
}

fn sort_key(cursor: ThreadCursor) -> (DateTime<Utc>, i64) {
    (cursor.last_at, cursor.id)
}

/// Three statements per batch, with an `IN` list well under the engine's parameter limit.
const COUNT_BATCH: usize = 400;

#[derive(Debug, Clone)]
struct Partner {
    thread: Thread,
    at: ThreadCursor,
}

fn prefixed_thread_columns(alias: &str) -> String {
    THREAD_COLUMNS
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ThreadListQuery {
    pub fn account(account_id: AccountId) -> Self {
        Self {
            account_id,
            mailbox: None,
            limit: DEFAULT_THREAD_PAGE_SIZE,
            after: None,
        }
    }

    pub fn in_mailbox(account_id: AccountId, mailbox: MailboxId) -> Self {
        Self {
            account_id,
            mailbox: Some(mailbox),
            limit: DEFAULT_THREAD_PAGE_SIZE,
            after: None,
        }
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    pub fn after(mut self, cursor: ThreadCursor) -> Self {
        self.after = Some(cursor);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusListQuery {
    pub inboxes: Vec<(AccountId, MailboxId)>,
    pub limit: u32,
    pub after: Option<ThreadCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListRow {
    /// `None` for a message in no thread: threading can fail (the send path ignores the result),
    /// and a list that hides mail because a derived column is null is worse than one that shows it
    /// ungrouped, so it is a conversation of one.
    pub id: Option<ThreadId>,
    pub subject: Option<String>,
    pub participants: Vec<EmailAddress>,
    /// Never scoped, even folder-scoped: the badge is the conversation's size (ADR 0015 Q2).
    pub message_count: u32,
    /// In a folder, this folder's slice.
    pub unread_count: u32,
    pub has_attachments: bool,
    pub is_flagged: bool,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    /// A reply filed in Archive is not what the Inbox row should show. `None` only when every
    /// message is hidden.
    pub latest: Option<MessageListRow>,
    /// Thread id in an account-scoped list, representative message id in a folder-scoped one; a
    /// cursor compares only against its own column.
    pub sort_id: i64,
    /// Read by Focus's pages only, in one statement per page; other lists leave it `None` and read
    /// nothing.
    pub marker: Option<MarkerSummary>,
    /// Replies filed away since the person muted the conversation (spec 013). Focus's pages only,
    /// one statement per page ([`ThreadRepository::muted`]).
    pub muted: Option<u32>,
}

impl ThreadListRow {
    pub fn cursor(&self) -> ThreadCursor {
        ThreadCursor {
            last_at: self.last_at,
            id: self.sort_id,
        }
    }

    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }
}

#[derive(Debug)]
pub struct ThreadRepository<'a> {
    connection: &'a Connection,
}

const THREAD_COLUMNS: &str = "\
id, account_id, subject, message_count, unread_count, has_attachments, is_flagged,
first_at, last_at";

/// A message hidden pending a remote delete, or snoozed and not due, is not a member: the list does
/// not show it, so it must not count. Written without an alias on the snooze half since
/// `snoozed_until` names no other table here. Same predicate as the folder counts
/// ([`super::VISIBLE`]), so list and counts cannot drift.
const MEMBER: &str = super::VISIBLE;

/// Folder asks [`MEMBER`] only; Focus adds [`super::focus_excludes`], the one predicate that takes
/// held mail out of the window, representative, slice, count, seek marks and changed-message rows
/// together (spec 007, research R13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Membership {
    Folder,
    Focus,
}

impl Membership {
    fn test(self, alias: &str) -> String {
        format!("{alias}{MEMBER}{}", self.excludes(alias))
    }

    /// For a statement asking [`MEMBER`] of a wider set and this membership of part of it, as a
    /// folded row's unread count does.
    fn excludes(self, alias: &str) -> String {
        match self {
            Membership::Folder => String::new(),
            Membership::Focus => super::focus_excludes(alias),
        }
    }
}

/// The same predicate the window uses, so number and rows cannot disagree.
fn folder_count_sql(membership: Membership) -> String {
    // A lone message stands for itself under its negated id. Read entirely from
    // `idx_messages_mailbox_threads`: the old per-message newer-member check took 786 ms on a
    // 60,907-message folder (#1534, #1607).
    format!(
        "SELECT count(DISTINCT coalesce(thread_id, -id)) FROM messages
          WHERE mailbox_id = ?1 AND {}",
        membership.test("")
    )
}

/// A message no newer member of its conversation here outranks: the representative the window draws
/// a row from. `also` narrows candidates before the `NOT EXISTS`.
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

/// Spelled once so the row's aggregates cannot drift apart on what counts as a member.
fn slice(membership: Membership, mailbox: &str) -> String {
    format!(
        "FROM messages m
              WHERE m.thread_id = rep.thread_id AND m.mailbox_id = {mailbox} AND {}",
        membership.test("m.")
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marked {
    pub threads: Vec<ThreadId>,
    pub lone: Vec<MessageId>,
    pub copies: HashMap<ThreadId, Vec<(AccountId, ThreadId)>>,
}

impl Marked {
    pub fn len(&self) -> u32 {
        (self.threads.len() + self.lone.len()) as u32
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.lone.is_empty()
    }
}

const FOCUS_REPRESENTATIVE: usize = 10;

/// The representative's list row is read in the same statement. Sort key and id are named so a
/// merge of several can order by them.
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
            muted: None,
        },
    ))
}

const WEEK: i64 = 7 * 24 * 60 * 60 * 1000;

impl<'a> ThreadRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The aggregates become true once messages are added, since every mutation recomputes them.
    pub async fn create(&self, thread: &mut Thread) -> Result<ThreadId> {
        let account_id = require_persisted(thread.account_id.get(), "account")?;
        // Cached: a first sync creates a thread for most messages it files (#728).
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

    /// Membership is not part of this: use [`ThreadRepository::add_message`].
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

    /// Its messages survive with no thread: threading can simply run again.
    pub async fn delete(&self, id: ThreadId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM threads WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    /// A previous thread is recomputed too: an abandoned thread still claiming the message would
    /// show a count the drill-in cannot produce.
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

    pub async fn recompute(&self, id: ThreadId) -> Result<()> {
        recompute_in(self.connection, id).await
    }

    /// One `EXISTS`: a seek on `idx_messages_thread_mailbox`, stopping at the first Sent message.
    pub async fn took_part(&self, id: ThreadId) -> Result<bool> {
        sql::exists(self.connection, Self::explain_took_part(), [id.get()]).await
    }

    pub fn explain_took_part() -> &'static str {
        "SELECT 1 FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
          WHERE m.thread_id = ?1 AND b.role = 'sent' LIMIT 1"
    }

    /// What a late-arriving parent does. Merging into the older thread keeps the id the UI may
    /// already show.
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

    /// What a surfaced reminder's row draws as its first line. One statement, a seek on
    /// `idx_messages_thread`.
    pub async fn latest_member(&self, id: ThreadId) -> Result<Option<MessageId>> {
        sql::first(
            self.connection,
            &Self::explain_latest_member(),
            [id.get()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    pub fn explain_latest_member() -> String {
        format!(
            "SELECT messages.id FROM messages
              WHERE messages.thread_id = ?1 AND messages.{MEMBER}
              ORDER BY messages.received_at DESC, messages.id DESC LIMIT 1"
        )
    }

    /// Where a row surfaced at `at` goes in Focus's inbox: how many conversations are newer (a
    /// conversation is newer when any member is). One statement: a seek on `idx_messages_list` to
    /// rows newer than `at`.
    ///
    /// With several accounts a conversation reaching two inboxes counts in each, so the row can sit
    /// one place lower than the fold would put it.
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

    pub fn explain_focus_position(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND {} AND messages.sort_at > ?{}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages."),
            inboxes + 1
        )
    }

    /// A draft is not included: this is what a verb on the thread acts on, and archiving a
    /// conversation leaves its unsent reply in Drafts (spec 013).
    pub async fn messages(&self, id: ThreadId, order: ThreadOrder) -> Result<Vec<MessageListRow>> {
        sql::all(
            self.connection,
            &self.explain_messages(order),
            [id.get()],
            read_list_row,
        )
        .await
    }

    /// What waking a snoozed conversation has to reach.
    pub async fn snoozed_messages(&self, id: ThreadId) -> Result<Vec<MessageListRow>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {LIST_COLUMNS} FROM messages
                  WHERE messages.thread_id = ?1 AND messages.snoozed_until IS NOT NULL
                  ORDER BY messages.received_at ASC, messages.id ASC"
            ),
            [id.get()],
            read_list_row,
        )
        .await
    }

    pub fn explain_messages(&self, order: ThreadOrder) -> String {
        let direction = match order {
            ThreadOrder::Oldest => "ASC",
            ThreadOrder::Newest => "DESC",
        };
        format!(
            "SELECT {LIST_COLUMNS} FROM messages
              WHERE messages.thread_id = ?1 AND messages.{MEMBER} AND messages.draft = 0
              ORDER BY messages.received_at {direction}, messages.id {direction}"
        )
    }

    /// Only inboxes (#1692): a conversation filed away is not a row, so a verb that files the
    /// cursor's row takes it out of the view and `a a a` walks down the list. Each inbox
    /// contributes its own folder-list rows; the order is global since message ids are unique
    /// across accounts, so windows merge by it and one cursor resumes all, reading `limit` rows per
    /// inbox.
    ///
    /// Grouping: a thread never spans accounts, but the list folds threads into one [`ThreadGroup`]
    /// when the conversation is one to the user: by root identity (another account holds a copy of
    /// the JWZ root, via `idx_messages_rfc_message_id`), or by matching normalised subject within
    /// [`postio_model::subject::COALESCING_WINDOW_DAYS`] (a bare subject match would fold every
    /// "Weekly digest" into one eternal row).
    ///
    /// Only a partner in view counts. A row is emitted at the group's newest member, so no
    /// conversation is two rows. Dedupe is display-only (Q13): `message_count` counts distinct
    /// `RfcMessageId`s, both copies stay, and [`ThreadGroup::members`] is what an action must
    /// expand to.
    pub async fn unified_page(&self, query: &UnifiedThreadListQuery) -> Result<Vec<ThreadGroup>> {
        let inboxes = self.unified_inboxes().await?;
        self.folded_page(Membership::Folder, &inboxes, query.limit, query.after)
            .await
    }

    /// The grouping [`Self::unified_page`] describes, for the unified inbox and Focus's (whose raw
    /// window is its own merged statement). A Focus row's participants are left to its caller.
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

        // The raw page over-fetches: absorption folds rows together, so rows can under-fill the
        // page of groups. Loop until groups fill or the list ends.
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

    /// By role, never by name, one per account (first by path, as
    /// [`MailboxRepository::by_role`](super::MailboxRepository::by_role)), so the folder a verb
    /// routes to is the one listed. A retired folder is no inbox, and an account with no inbox yet
    /// contributes nothing.
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

    /// Driven from `accounts` into `idx_mailboxes_account_role`, so no statement reads mail.
    pub fn explain_unified_inboxes(&self) -> String {
        "SELECT m.account_id, m.id
           FROM accounts a JOIN mailboxes m
             ON m.account_id = a.id AND m.role = 'inbox'
          WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1
          ORDER BY m.account_id, m.path"
            .to_owned()
    }

    /// The same bargain as [`page_at`](Self::page_at): the grouping walk only resumes from a
    /// cursor, so it over-fetches by `offset` and drops the head, since absorption means a group is
    /// not a fixed number of rows. `postio_runtime::store`'s seek marks keep it small.
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

    /// The inboxes' own counts less the rows absorption folds away. With one inbox nothing folds.
    /// With more, the fold uses the page's rule over every inbox conversation, so count and walk
    /// agree; bounded by what the inboxes hold, and `postio_runtime::store` holds the answer until
    /// an inbox moves.
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

    /// The rows the walk in [`Self::folded_page`] skips because a partner in view is newer, by the
    /// same rule, so count and walk agree.
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

    /// Each inbox is asked for its own `limit` rows after the cursor; the union's first `limit` are
    /// exactly the merged list's.
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

    /// Each group of copies of one conversation is one row, by the rule [`Self::folded_page`] folds
    /// by; members come newest first, the first being the one the row is drawn from. Five
    /// statements with several inboxes, none with one.
    async fn fold_in_view(
        &self,
        membership: Membership,
        inboxes: &[(AccountId, MailboxId)],
        counted: &[(AccountId, ThreadId)],
    ) -> Result<Vec<Vec<(AccountId, ThreadId)>>> {
        if inboxes.len() < 2 || counted.is_empty() {
            return Ok(counted.iter().map(|member| vec![*member]).collect());
        }
        let ids: Vec<ThreadId> = counted.iter().map(|(_, thread)| *thread).collect();
        let threads = self.threads_by_id(&ids).await?;
        let candidates = self.group_partners_for(&threads).await?;
        let mut accounts: HashMap<ThreadId, AccountId> = counted
            .iter()
            .map(|(account, thread)| (*thread, *account))
            .collect();
        for partner in candidates.values().flatten() {
            accounts.entry(partner.id).or_insert(partner.account_id);
        }
        let mut wanted: Vec<ThreadId> = accounts.keys().copied().collect();
        wanted.sort_unstable();
        let keys = self.keys_in_view(membership, inboxes, &wanted).await?;

        // Union the copies in view, each conversation first its own root.
        let mut root: HashMap<ThreadId, ThreadId> = HashMap::new();
        fn find(root: &mut HashMap<ThreadId, ThreadId>, thread: ThreadId) -> ThreadId {
            let parent = *root.entry(thread).or_insert(thread);
            if parent == thread {
                return thread;
            }
            let top = find(root, parent);
            root.insert(thread, top);
            top
        }
        for thread in &ids {
            find(&mut root, *thread);
        }
        for (thread, partners) in &candidates {
            for partner in partners {
                if keys.contains_key(&partner.id) {
                    let (a, b) = (find(&mut root, *thread), find(&mut root, partner.id));
                    if a != b {
                        root.insert(a, b);
                    }
                }
            }
        }
        let mut rows: HashMap<ThreadId, Vec<ThreadId>> = HashMap::new();
        let members: Vec<ThreadId> = root.keys().copied().collect();
        for thread in members {
            let top = find(&mut root, thread);
            rows.entry(top).or_default().push(thread);
        }
        let newest = |thread: &ThreadId| keys.get(thread).map(|key| sort_key(*key));
        let mut folded: Vec<Vec<(AccountId, ThreadId)>> = rows
            .into_values()
            .map(|mut members| {
                members.sort_by_key(|thread| std::cmp::Reverse((newest(thread), thread.get())));
                members
                    .into_iter()
                    .map(|thread| (accounts[&thread], thread))
                    .collect()
            })
            .collect();
        folded.sort_by_key(|members| std::cmp::Reverse(newest(&members[0].1)));
        Ok(folded)
    }

    /// One statement, sought through the conversations' own index.
    async fn keys_in_view(
        &self,
        membership: Membership,
        inboxes: &[(AccountId, MailboxId)],
        wanted: &[ThreadId],
    ) -> Result<HashMap<ThreadId, ThreadCursor>> {
        if wanted.is_empty() || inboxes.is_empty() {
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

        Ok(keys)
    }

    /// A partner filed away neither takes the row nor folds into it, and an action does not reach
    /// it through the group.
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

        let keys = self.keys_in_view(membership, inboxes, &wanted).await?;

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

    /// What a whole-view selection over Focus's inbox takes back out when a row is deselected (spec
    /// 007, T167): every copy has to stay with the row.
    pub async fn with_folded_copies(&self, threads: &[ThreadId]) -> Result<Vec<ThreadId>> {
        let found = self.threads_by_id(threads).await?;
        let partners = self.group_partners_for(&found).await?;
        let mut all: Vec<ThreadId> = threads.to_vec();
        for copies in partners.values() {
            all.extend(copies.iter().map(|thread| thread.id));
        }
        all.sort();
        all.dedup();
        Ok(all)
    }

    /// Three statements for the whole page, not per thread: the per-thread version was the unified
    /// page's entire cost.
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
            // The common group (one thread, one account): its inbox row is the answer; deduping a
            // set of one was most of the unified page's cost.
            return Ok(row);
        }

        // Distinct messages, not rows: a copy received at two addresses is one message, and one
        // with no RfcMessageId counts by row. The badge is the conversation's size wherever filed;
        // unread is the inboxes' slice.
        let mut members: Vec<ThreadId> = vec![head];
        members.extend(partners.iter().map(|partner| partner.thread.id));
        (row.message_count, row.unread_count) =
            self.folded_counts(membership, &members, inboxes).await?;
        for partner in partners {
            row.has_attachments |= partner.thread.has_attachments;
            row.is_flagged |= partner.thread.is_flagged;
            row.first_at = row.first_at.min(partner.thread.first_at);
        }
        Ok(row)
    }

    /// Distinct messages, not rows (a copy at two addresses is one; no RfcMessageId counts by row).
    /// Badge is the conversation's size, unread the inboxes' slice. One statement.
    async fn folded_counts(
        &self,
        membership: Membership,
        members: &[ThreadId],
        inboxes: &[(AccountId, MailboxId)],
    ) -> Result<(u32, u32)> {
        let first_inbox = members.len() + 1;
        sql::one(
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
                .map(|thread| thread.get())
                .chain(inboxes.iter().map(|(_, inbox)| inbox.get()))
                .collect::<Vec<_>>(),
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await
    }

    /// One account: two statements. The window is one statement: each inbox's folder window over
    /// `idx_messages_list`, merged by `(sort_at, id)` (total, as message ids are unique across
    /// accounts); the merge sorts at most inboxes times the page. The representative's list row
    /// comes in the same row (`contracts/engine.md`). Participants are the second, markers the
    /// third, and a page with an invitation a fourth for its clashes.
    ///
    /// Several: a conversation received at two addresses is one row by the partner search of
    /// [`Self::folded_page`], and [`ThreadGroup::members`] names both copies. With one inbox the
    /// search is not asked; with more a page adds five statements and one per folded row's counts.
    ///
    /// `offset` counts rows from the cursor, with the caveat [`Self::page_at`] carries.
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
            // As in `unified_page_at`: only the walk that produced the earlier rows knows where the
            // offset-th row starts, since a group is not a fixed number of raw rows.
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
            if row.participants.is_empty()
                && let Some(from) = row.latest.as_ref().and_then(|latest| latest.from.clone())
            {
                row.participants = vec![from];
            }
        }
        self.mark(&query.inboxes, &mut groups).await?;
        self.mute_marks(&mut groups).await?;
        Ok(groups)
    }

    /// One statement for the page, none without a conversation. A folded row is muted when any copy
    /// is.
    async fn mute_marks(&self, groups: &mut [ThreadGroup]) -> Result<()> {
        let mut threads: Vec<ThreadId> = groups
            .iter()
            .flat_map(|group| group.members.iter().map(|(_, thread)| *thread))
            .collect();
        threads.sort_unstable();
        threads.dedup();
        let muted = self.muted(&threads).await?;
        if muted.is_empty() {
            return Ok(());
        }
        for ThreadGroup { row, members } in groups {
            let replies: Vec<u32> = members
                .iter()
                .filter_map(|(_, thread)| muted.get(thread).copied())
                .collect();
            row.muted = (!replies.is_empty()).then(|| replies.iter().sum());
        }
        Ok(())
    }

    /// One statement for the page ([`Self::explain_focus_markers`]). A folded row draws its own
    /// conversation's marker first and a copy's only if its own has none: the copies are one
    /// conversation.
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
            let mut marker = read_marker_summary(row, 2)?;
            let uid = row.col::<Option<String>>(10)?;
            Ok((row.col::<Option<i64>>(0)?, row.col::<i64>(1)?, {
                marker.clash = uid;
                marker
            }))
        })
        .await?;
        drop(statement);
        let found = self.clashes(found).await?;

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

    async fn clashes(
        &self,
        mut found: Vec<(Option<i64>, i64, MarkerSummary)>,
    ) -> Result<Vec<(Option<i64>, i64, MarkerSummary)>> {
        let mut asked = Vec::new();
        let mut arms = Vec::new();
        let mut arguments: Vec<turso::Value> = Vec::new();
        for (at, (_, _, marker)) in found.iter_mut().enumerate() {
            let uid = marker.clash.take();
            let (MarkerKind::Invite, Some(MarkerWhen::Event { starts_at, ends_at })) =
                (marker.kind, marker.when.as_ref())
            else {
                continue;
            };
            if marker.cancelled {
                continue;
            }
            let base = arguments.len();
            arguments.extend([
                to_millis(*starts_at).bind(),
                to_millis(*ends_at).bind(),
                uid.unwrap_or_default().bind(),
            ]);
            arms.push(format!(
                "SELECT {at}, (SELECT e.title FROM event_occurrences o JOIN events e ON e.id = o.event_id
                   WHERE o.start_utc >= ?{a} - {WEEK} AND o.start_utc < ?{b} AND o.end_utc > ?{a}
                     AND o.all_day = 0 AND e.status <> 'cancelled'
                     AND coalesce(e.my_partstat, '') <> 'DECLINED' AND e.uid <> ?{c}
                   ORDER BY o.start_utc LIMIT 1)",
                a = base + 1,
                b = base + 2,
                c = base + 3,
            ));
            asked.push(at);
        }
        if asked.is_empty() {
            return Ok(found);
        }
        let titles: Vec<(i64, Option<String>)> = sql::all(
            self.connection,
            &arms.join("\n UNION ALL\n"),
            arguments,
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        for (at, title) in titles {
            if let Some((_, _, marker)) = usize::try_from(at).ok().and_then(|at| found.get_mut(at))
            {
                marker.clash = title;
            }
        }
        Ok(found)
    }

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

    /// The Focus twin of [`Self::unified_raw_page`], which asks each inbox in turn; this merges in
    /// one statement.
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
        // Each inbox is asked for as many rows as the page could need; the merge keeps the page's.
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

    /// Per conversation, its newest message in the inboxes' Focus slice carrying an undismissed
    /// marker. Driven from the conversations' own index and into `markers` by key. `m.mailbox_id +
    /// 0` keeps it there: with the column bare the planner seeks the inbox and walks all of it. The
    /// newest marked message is `max()`'s row, whose bare columns the engine answers from (as
    /// [`Self::participants_for`] relies on with `min()`).
    pub fn explain_focus_markers(&self, inboxes: usize, threads: usize, lone: usize) -> String {
        let columns = "k.kind, k.starts_at, k.ends_at, k.due_at, k.excerpt, k.answer, \
                       k.invite_state, k.moved_from, k.invite_uid, k.source";
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

    /// Not held for a digest, not filtered away, not moved since: what a notification may be about.
    pub async fn stayed_in_focus(
        &self,
        mailbox: MailboxId,
        messages: &[MessageId],
    ) -> Result<Vec<MessageId>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let mut arguments = vec![mailbox.get()];
        arguments.extend(messages.iter().map(|message| message.get()));
        sql::all(
            self.connection,
            &self.explain_stayed_in_focus(messages.len()),
            arguments,
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    pub fn explain_stayed_in_focus(&self, messages: usize) -> String {
        format!(
            "SELECT messages.id FROM messages
              WHERE messages.id IN ({}) AND messages.mailbox_id = ?1 AND {}
                AND NOT EXISTS (SELECT 1 FROM filter_decisions d
                                 WHERE d.message_id = messages.id
                                   AND d.restored_at IS NULL)
              ORDER BY messages.id",
            placeholders(messages, 2),
            Membership::Focus.test("messages.")
        )
    }

    /// The first part is one statement from `idx_messages_mailbox_threads` alone: a conversation
    /// belongs to one account and each gives one inbox, so distinct keys across inboxes are the sum
    /// of each's own. With one inbox nothing folds; with more the fold is counted as in
    /// [`Self::unified_count`], over the same membership.
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

    pub fn explain_focus_count(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages.")
        )
    }

    /// One statement driven from the undismissed markers (a few hundred, where an inbox is a
    /// hundred thousand) through `idx_markers_dismissed`, then to each message by key. A marker
    /// counts where the row would draw it, so an archived, snoozed or held message's marker makes
    /// no row. `idx_markers_open` was planned as a partial index, which this planner does not read.
    pub async fn focus_marked(&self, inboxes: &[(AccountId, MailboxId)]) -> Result<Marked> {
        if inboxes.is_empty() {
            return Ok(Marked::default());
        }
        let arguments: Vec<i64> = inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        let found: Vec<(Option<i64>, i64, i64)> = sql::all(
            self.connection,
            &self.explain_focus_marked(inboxes.len()),
            arguments,
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
        )
        .await?;
        let mut marked = Marked::default();
        let mut conversations = Vec::new();
        for (thread, message, account) in found {
            match thread {
                Some(thread) => {
                    conversations.push((AccountId::new(account), ThreadId::new(thread)));
                }
                None => marked.lone.push(MessageId::new(message)),
            }
        }
        // A conversation received at two addresses is one row, drawn from the copy the inbox draws
        // it from (T168).
        for mut row in self
            .fold_in_view(Membership::Focus, inboxes, &conversations)
            .await?
        {
            let (_, drawn) = row.remove(0);
            marked.threads.push(drawn);
            if !row.is_empty() {
                marked.copies.insert(drawn, row);
            }
        }
        Ok(marked)
    }

    pub fn explain_focus_marked(&self, inboxes: usize) -> String {
        format!(
            "SELECT DISTINCT m.thread_id, CASE WHEN m.thread_id IS NULL THEN m.id ELSE 0 END,
                    m.account_id
               FROM markers k JOIN messages m ON m.id = k.message_id
              WHERE k.dismissed_at IS NULL AND m.mailbox_id IN ({}) AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("m.")
        )
    }

    /// Three statements, the inbox page's budget: the window (sought through the marked
    /// conversations' own index), the participants, the markers; plus one per folded row's counts
    /// (T168).
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
            .map(|(account, row)| {
                let copies = row
                    .id
                    .and_then(|thread| marked.copies.get(&thread))
                    .cloned()
                    .unwrap_or_default();
                ThreadGroup {
                    members: row
                        .id
                        .map(|thread| (account, thread))
                        .into_iter()
                        .chain(copies)
                        .collect(),
                    row,
                }
            })
            .collect();
        // A folded row counts its copies once, as the inbox's does.
        for ThreadGroup { row, members } in &mut groups {
            if members.len() > 1 {
                let threads: Vec<ThreadId> = members.iter().map(|(_, thread)| *thread).collect();
                (row.message_count, row.unread_count) = self
                    .folded_counts(Membership::Focus, &threads, inboxes)
                    .await?;
            }
        }

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

    /// One statement, sought through each inbox's index. With several accounts a conversation
    /// unread at two addresses is one row (T168): the unread conversations are folded by the page's
    /// partner search, five statements more.
    pub async fn focus_unread(&self, inboxes: &[(AccountId, MailboxId)]) -> Result<u32> {
        if inboxes.is_empty() {
            return Ok(0);
        }
        let arguments: Vec<i64> = inboxes.iter().map(|(_, inbox)| inbox.get()).collect();
        if inboxes.len() < 2 {
            let count: i64 = sql::one(
                self.connection,
                &self.explain_focus_unread(inboxes.len()),
                arguments,
                |row| row.col(0),
            )
            .await?;
            return Ok(count.max(0) as u32);
        }
        // Several inboxes: fold the unread conversations as the page does, so one unread at two
        // addresses counts once (T168).
        let unread: Vec<(i64, Option<i64>)> = sql::all(
            self.connection,
            &self.explain_focus_unread_rows(inboxes.len()),
            arguments,
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let lone = unread.iter().filter(|(_, thread)| thread.is_none()).count();
        let conversations: Vec<(AccountId, ThreadId)> = unread
            .iter()
            .filter_map(|(account, thread)| {
                thread.map(|thread| (AccountId::new(*account), ThreadId::new(thread)))
            })
            .collect();
        let rows = self
            .fold_in_view(Membership::Focus, inboxes, &conversations)
            .await?
            .len();
        Ok((lone + rows) as u32)
    }

    pub fn explain_focus_unread_rows(&self, inboxes: usize) -> String {
        format!(
            "SELECT DISTINCT messages.account_id,
                    messages.thread_id,
                    CASE WHEN messages.thread_id IS NULL THEN messages.id ELSE 0 END
               FROM messages
              WHERE messages.mailbox_id IN ({}) AND messages.seen = 0 AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages.")
        )
    }

    pub fn explain_focus_unread(&self, inboxes: usize) -> String {
        format!(
            "SELECT count(DISTINCT coalesce(messages.thread_id, -messages.id)) FROM messages
              WHERE messages.mailbox_id IN ({}) AND messages.seen = 0 AND {}",
            placeholders(inboxes, 1),
            Membership::Focus.test("messages.")
        )
    }

    pub async fn page(&self, query: &ThreadListQuery) -> Result<Vec<ThreadListRow>> {
        self.page_with(query, "").await
    }

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

    /// One row per touched conversation, in the page's own shape (#1607): a flag change names
    /// message ids, and re-reading a whole fifty-row page for one unread count was too costly. Two
    /// statements before the page's own two: which conversations the ids touch, then their
    /// representatives through the thread index. A message with no conversation is its own row,
    /// except in an account-wide list.
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

    /// An archive moves `mailboxes.total_count`, the folder count's witness, so the next page paid
    /// the whole conversation count again (786 ms on a real folder, #1607). What was lost is
    /// knowable from the removed ids alone: conversations with no member left in the folder plus
    /// every lone message, in two statements bounded by the ids.
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
                    muted: None,
                },
                // The representative's id, which the folder window already
                // knows and the account window has to look up.
                scoped.then(|| MessageId::new(row.col::<i64>(9).unwrap_or_default())),
            ))
        })
        .await?;
        let mut page: Vec<(ThreadListRow, Option<MessageId>)> = rows;
        drop(statement);

        let ids: Vec<ThreadId> = page.iter().filter_map(|(row, _)| row.id).collect();
        let mut participants = self.participants_for(&ids).await?;
        if scoped {
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

    /// The window is over `threads` by `last_at DESC, id DESC` on `idx_threads_account_last_at`, so
    /// it never sorts or scans. The folder contributes correlated subqueries per page row, not a
    /// join that widens the ordered set; each seeks `idx_messages_thread_mailbox` and is bounded by
    /// one conversation, so a page costs `limit` seeks plus a constant.
    /// `the_thread_list_plan_never_sorts` is the structural check, `store_reads` the empirical one.
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

    /// Prefer [`ThreadRepository::page`]: an offset counts from the top every time, so a
    /// conversation that moves while the user scrolls silently skips a row. The store's seek marks
    /// (`postio_runtime::store`) keep the offset small.
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

    /// What a scrollbar jump seeks from instead of skipping (#1610).
    pub async fn boundaries(
        &self,
        query: &ThreadListQuery,
        stride: u32,
    ) -> Result<Vec<(u32, ThreadCursor)>> {
        let Some(mailbox) = query.mailbox else {
            return Ok(Vec::new());
        };
        let stride = stride.max(1);
        // One pass over the folder's index, newest first, keys only: a list row is the first
        // message of its conversation met in that order, or a message in no conversation, so
        // counting as they go by counts rows.
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

    pub async fn count(&self, account_id: AccountId) -> Result<u32> {
        self.count_of(&ThreadListQuery::account(account_id)).await
    }

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

    pub fn explain_count_of(&self) -> String {
        folder_count_sql(Membership::Folder)
    }

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

    /// One statement: a seek on `idx_messages_thread` and a primary-key seek per member, whatever
    /// the length (spec 013 R6). `line` is the sender's own words when cut, [`Line::Quoted`] when
    /// empty, else the stored preview cut at an attribution. `answers` is set only for a parent
    /// that is not the member just before.
    pub async fn members(&self, id: ThreadId) -> Result<Vec<ThreadMember>> {
        let rows = sql::all(
            self.connection,
            &Self::explain_members(),
            [id.get()],
            |row| {
                let listed = read_list_row(row)?;
                let own_line: Option<String> = row.col(17)?;
                let rfc: Option<String> = row.col(18)?;
                let parent: Option<String> = row.col(19)?;
                let line = match own_line {
                    Some(line) if line.is_empty() => Line::Quoted,
                    Some(line) => Line::Own(line),
                    None => Line::Preview(
                        before_attribution(listed.preview.as_deref().unwrap_or("")).to_owned(),
                    ),
                };
                let member = ThreadMember {
                    id: listed.id,
                    from: listed.from,
                    own: row.col(21)?,
                    at: listed.received_at,
                    seen: listed.seen,
                    has_attachments: listed.has_attachments,
                    line,
                    answers: None,
                    marker: row.col(20)?,
                };
                Ok((member, rfc, parent))
            },
        )
        .await?;
        let mut at: HashMap<String, usize> = HashMap::with_capacity(rows.len());
        for (index, (_, rfc, _)) in rows.iter().enumerate() {
            if let Some(rfc) = rfc {
                at.entry(RfcMessageId::new(rfc).folded()).or_insert(index);
            }
        }
        let parents: Vec<Option<usize>> = rows
            .iter()
            .map(|(_, _, parent)| {
                parent
                    .as_ref()
                    .and_then(|parent| at.get(&RfcMessageId::new(parent).folded()).copied())
            })
            .collect();
        Ok(rows
            .into_iter()
            .zip(parents)
            .enumerate()
            .map(|(index, ((mut member, _, _), parent))| {
                member.answers = parent
                    .filter(|&parent| parent != index && parent + 1 != index)
                    .and_then(|parent| u32::try_from(parent).ok());
                member
            })
            .collect())
    }

    pub fn explain_members() -> String {
        format!(
            "SELECT {LIST_COLUMNS}, messages.own_line, messages.rfc_message_id, messages.in_reply_to,
                EXISTS (SELECT 1 FROM markers k
                         WHERE k.message_id = messages.id AND k.dismissed_at IS NULL),
                EXISTS (SELECT 1 FROM recipients r JOIN addresses a ON a.id = r.address_id
                         WHERE r.message_id = messages.id AND r.kind = 'from'
                           AND (a.address_normalized IN
                                  (SELECT lower(i.address) FROM identities i
                                    WHERE i.account_id = messages.account_id)
                                OR a.address_normalized =
                                  (SELECT lower(c.address) FROM accounts c
                                    WHERE c.id = messages.account_id)))
               FROM messages
              WHERE messages.thread_id = ?1 AND messages.{MEMBER} AND messages.draft = 0
              ORDER BY messages.received_at, messages.id"
        )
    }

    pub async fn account_of(&self, id: ThreadId) -> Result<Option<AccountId>> {
        sql::first(
            self.connection,
            "SELECT account_id FROM threads WHERE id = ?1",
            [id.get()],
            |row| Ok(AccountId::new(row.col(0)?)),
        )
        .await
    }

    pub async fn thread_id_of(&self, id: MessageId) -> Result<Option<ThreadId>> {
        thread_of(self.connection, id).await
    }

    /// The oldest message's `Message-ID` (the root, research R3) or any claim of the thread; `None`
    /// when it has none.
    async fn root_claim(&self, id: ThreadId) -> Result<Option<(AccountId, String)>> {
        let Some(account) = sql::first(
            self.connection,
            "SELECT account_id FROM threads WHERE id = ?1",
            [id.get()],
            |row| Ok(AccountId::new(row.col(0)?)),
        )
        .await?
        else {
            return Err(Error::NotFound {
                entity: "thread",
                id: id.get(),
            });
        };
        // The claim must be one the thread holds, or `muted` would never find
        // it: the oldest message's id, when the thread claims it.
        let claim = sql::first(
            self.connection,
            "SELECT l.rfc_message_id FROM messages m
               JOIN thread_links l ON l.account_id = m.account_id
                                  AND l.rfc_message_id = lower(trim(m.rfc_message_id))
              WHERE m.thread_id = ?1 AND l.thread_id = ?1
              ORDER BY m.received_at, m.id LIMIT 1",
            [id.get()],
            |row| row.col::<String>(0),
        )
        .await?;
        let claim = match claim {
            Some(claim) => Some(claim),
            None => {
                sql::first(
                    self.connection,
                    "SELECT rfc_message_id FROM thread_links WHERE thread_id = ?1
                      ORDER BY rfc_message_id LIMIT 1",
                    [id.get()],
                    |row| row.col::<String>(0),
                )
                .await?
            }
        };
        Ok(claim.map(|claim| (account, claim)))
    }

    /// Written against a claimed `Message-ID`, not the thread, so the mute survives a merge and
    /// renumbering (research R3). Muting a muted thread changes nothing.
    pub async fn mute(&self, id: ThreadId) -> Result<()> {
        if !self.muted(&[id]).await?.is_empty() {
            return Ok(());
        }
        let Some((account, claim)) = self.root_claim(id).await? else {
            return Err(Error::ForbiddenTransition {
                what: "mute",
                reason: "the thread claims no Message-ID to remember the mute by".to_owned(),
            });
        };
        sql::execute(
            self.connection,
            "INSERT OR REPLACE INTO thread_mutes (account_id, rfc_message_id, muted_at, replies_since)
             VALUES (?1, ?2, ?3, 0)",
            bind![account.get(), claim, to_millis(Utc::now())],
        )
        .await?;
        Ok(())
    }

    pub async fn unmute(&self, id: ThreadId) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM thread_mutes
              WHERE account_id = (SELECT account_id FROM threads WHERE id = ?1)
                AND rfc_message_id IN
                    (SELECT rfc_message_id FROM thread_links WHERE thread_id = ?1)",
            [id.get()],
        )
        .await?;
        Ok(())
    }

    /// Driven from `thread_mutes` (a few threads) into `thread_links` by its unique key, so the
    /// rows read are the mutes however long the threads. A thread holding two mutes answers their
    /// sum.
    pub async fn muted(&self, ids: &[ThreadId]) -> Result<HashMap<ThreadId, u32>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let found = sql::all(
            self.connection,
            &Self::explain_muted(ids.len()),
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            |row| Ok((ThreadId::new(row.col(0)?), row.col::<u32>(1)?)),
        )
        .await?;
        let mut muted = HashMap::with_capacity(found.len());
        for (thread, replies) in found {
            *muted.entry(thread).or_insert(0) += replies;
        }
        Ok(muted)
    }

    pub fn explain_muted(threads: usize) -> String {
        format!(
            "SELECT l.thread_id, m.replies_since
               FROM thread_mutes m
               CROSS JOIN thread_links l
                 ON l.account_id = m.account_id AND l.rfc_message_id = m.rfc_message_id
              WHERE l.thread_id IN ({})",
            placeholders(threads, 1)
        )
    }

    pub async fn note_muted_replies(&self, id: ThreadId, replies: u32) -> Result<()> {
        let key = sql::first(
            self.connection,
            "SELECT m.account_id, m.rfc_message_id
               FROM thread_mutes m
               CROSS JOIN thread_links l
                 ON l.account_id = m.account_id AND l.rfc_message_id = m.rfc_message_id
              WHERE l.thread_id = ?1 LIMIT 1",
            [id.get()],
            |row| Ok((row.col::<i64>(0)?, row.col::<String>(1)?)),
        )
        .await?;
        if let Some((account, claim)) = key {
            sql::execute(
                self.connection,
                "UPDATE thread_mutes SET replies_since = replies_since + ?3
                  WHERE account_id = ?1 AND rfc_message_id = ?2",
                bind![account, claim, replies],
            )
            .await?;
        }
        Ok(())
    }

    /// Takes the message and every reply to it into a new thread and remembers it (spec 013): a
    /// later message naming one joins the new thread, nothing merges them back. One transaction;
    /// members are read once (ids and references, no bodies).
    ///
    /// Refused when `message` has no `Message-ID` to be remembered by, or when everything answers
    /// it (nothing would stay).
    pub async fn split(&self, message: MessageId) -> Result<ThreadId> {
        sql::in_scope(self.connection, |transaction| async move {
            let Some((original, account, own)) = sql::first(
                &transaction,
                "SELECT thread_id, account_id, rfc_message_id FROM messages WHERE id = ?1",
                [message.get()],
                |row| {
                    Ok((
                        row.col::<Option<i64>>(0)?.map(ThreadId::new),
                        AccountId::new(row.col(1)?),
                        row.col::<Option<String>>(2)?,
                    ))
                },
            )
            .await?
            else {
                return Err(Error::NotFound {
                    entity: "message",
                    id: message.get(),
                });
            };
            let (Some(original), Some(own)) = (original, own) else {
                return Err(Error::ForbiddenTransition {
                    what: "split",
                    reason: "the message has no thread, or no Message-ID to remember a split by"
                        .to_owned(),
                });
            };
            let nodes = sql::all(
                &transaction,
                "SELECT id, rfc_message_id, in_reply_to, reference_ids FROM messages
                  WHERE thread_id = ?1 AND draft = 0 ORDER BY received_at, id",
                [original.get()],
                |row| {
                    let id: Option<String> = row.col(1)?;
                    let in_reply_to: Option<String> = row.col(2)?;
                    let references: String = row.col(3)?;
                    let mut named: Vec<RfcMessageId> = references
                        .split_whitespace()
                        .map(RfcMessageId::new)
                        .collect();
                    named.extend(in_reply_to.map(RfcMessageId::new));
                    Ok(ThreadNode {
                        id: MessageId::new(row.col(0)?),
                        message_id: id.map(RfcMessageId::new),
                        references: named,
                    })
                },
            )
            .await?;
            let moved = descendants(&nodes, message);
            if moved.len() == nodes.len() {
                return Err(Error::ForbiddenTransition {
                    what: "split",
                    reason: "every message of the thread answers it, so none would stay".to_owned(),
                });
            }

            let mut thread = Thread::new(account);
            let threads = ThreadRepository::new(&transaction);
            thread.subject = threads.subject(original).await?.flatten();
            let split = threads.create(&mut thread).await?;

            let ids: Vec<i64> = moved.iter().map(|id| id.get()).collect();
            let mut parameters = vec![turso::Value::from(split.get())];
            parameters.extend(ids.iter().map(|id| turso::Value::from(*id)));
            sql::execute(
                &transaction,
                &format!(
                    "UPDATE messages SET thread_id = ?1 WHERE id IN ({})",
                    placeholders(ids.len(), 2)
                ),
                parameters,
            )
            .await?;

            // Their own ids move with them; the ids they merely name stay
            // with the thread they answer.
            let claims: Vec<String> = nodes
                .iter()
                .filter(|node| moved.contains(&node.id))
                .filter_map(|node| node.message_id.as_ref().map(RfcMessageId::folded))
                .collect();
            let mut parameters = vec![
                turso::Value::from(split.get()),
                turso::Value::from(account.get()),
                turso::Value::from(original.get()),
            ];
            parameters.extend(claims.iter().cloned().map(turso::Value::from));
            sql::execute(
                &transaction,
                &format!(
                    "UPDATE thread_links SET thread_id = ?1
                      WHERE account_id = ?2 AND thread_id = ?3 AND rfc_message_id IN ({})",
                    placeholders(claims.len(), 4)
                ),
                parameters,
            )
            .await?;
            sql::execute(
                &transaction,
                "INSERT OR REPLACE INTO thread_splits (account_id, rfc_message_id, split_at)
                 VALUES (?1, ?2, ?3)",
                bind![
                    account.get(),
                    RfcMessageId::new(&own).folded(),
                    to_millis(Utc::now())
                ],
            )
            .await?;

            recompute_in(&transaction, split).await?;
            recompute_in(&transaction, original).await?;
            Ok(split)
        })
        .await
    }

    /// What `z` does after a split: forgets the split record so a later reply joins `into` again.
    pub async fn unsplit(&self, split: ThreadId, into: ThreadId) -> Result<()> {
        if split == into {
            return Ok(());
        }
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "DELETE FROM thread_splits
                  WHERE account_id = (SELECT account_id FROM threads WHERE id = ?1)
                    AND rfc_message_id IN
                        (SELECT rfc_message_id FROM thread_links WHERE thread_id = ?1)",
                [split.get()],
            )
            .await?;
            // Relink before the merge deletes the thread, as threading does.
            sql::execute(
                &transaction,
                "UPDATE OR REPLACE thread_links SET thread_id = ?2 WHERE thread_id = ?1",
                bind![split.get(), into.get()],
            )
            .await?;
            ThreadRepository::new(&transaction).merge(into, split).await
        })
        .await
    }

    /// `Some(None)` when the thread has no subject. A rowid seek, none of [`Self::get`]'s member
    /// reads.
    pub async fn subject(&self, id: ThreadId) -> Result<Option<Option<String>>> {
        sql::first(
            self.connection,
            "SELECT subject FROM threads WHERE id = ?1",
            [id.get()],
            |row| row.col(0),
        )
        .await
    }

    pub async fn draft(&self, id: ThreadId) -> Result<Option<ThreadDraft>> {
        sql::first(self.connection, Self::explain_draft(), [id.get()], |row| {
            let text: Option<String> = row.col(1)?;
            let html: Option<String> = row.col(2)?;
            let words = text
                .filter(|text| !text.trim().is_empty())
                .or(html)
                .unwrap_or_default();
            Ok(ThreadDraft {
                id: DraftId::new(row.col(0)?),
                first_words: preview(&words).unwrap_or_default(),
                updated_at: from_millis(row.col(3)?),
            })
        })
        .await
    }

    pub fn explain_draft() -> &'static str {
        "SELECT id, body_text, body_html, updated_at FROM drafts
          WHERE thread_id = ?1 AND state <> 'sent'
          ORDER BY updated_at DESC, id DESC LIMIT 1"
    }

    /// One statement for the page. `min(received_at)` orders them, and SQLite takes the bare
    /// `name`/`address` from that row, so the display name is the one first used.
    async fn participants_for(
        &self,
        ids: &[ThreadId],
    ) -> Result<HashMap<ThreadId, Vec<EmailAddress>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "SELECT messages.thread_id, coalesce({shown}, recipients.name),
                    addresses.address, min(messages.received_at) AS first_seen
               FROM messages
               JOIN recipients ON recipients.message_id = messages.id
               JOIN addresses ON addresses.id = recipients.address_id
               LEFT JOIN contacts named
                 ON named.id = addresses.contact_id AND named.state = 'live'
              WHERE messages.thread_id IN ({}) AND messages.{MEMBER}
                AND recipients.kind = 'from'
              GROUP BY messages.thread_id, recipients.address_id
              ORDER BY messages.thread_id, first_seen, recipients.id",
            placeholders(ids.len(), 1),
            shown = crate::shown_name!("named")
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

    /// One statement: `row_number()` picks the newest per thread, and sender lookups run only for
    /// survivors.
    async fn latest_messages_for(
        &self,
        ids: &[ThreadId],
        mailbox: Option<MailboxId>,
    ) -> Result<HashMap<ThreadId, MessageListRow>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        // The folder narrows what the window function ranks; still one statement.
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
                // `LIST_COLUMNS`, not a copy: a hand-written copy fell behind when a column was
                // added, and the unified list failed to page while every storage test passed.
                // `thread_id` is the second column in both.
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

/// The subject is the oldest member's, recomputed since a merge can change which that is.
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
        clash: None,
        moved_from: row.col::<Option<i64>>(first + 7)?.map(from_millis),
        notice: row.col::<String>(first + 9)? == "notice",
    })
}

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
