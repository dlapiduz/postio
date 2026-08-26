//! Messages, and the windowed query the message list pages through.
//!
//! # The list must never load a mailbox
//!
//! docs/PRODUCT.md §18 and the `<16 ms` interaction budget in CLAUDE.md make this
//! structural rather than aspirational: the only way to read more than one
//! message out of this repository is [`MessageRepository::page`], which returns
//! at most [`ListQuery::limit`] rows of exactly the fields a list row renders.
//! There is deliberately no `all()`.
//!
//! # Why a cursor and not an offset
//!
//! Paging by `OFFSET` counts rows from the top every time, so it is `O(offset)`
//! *and* it skips a row whenever a message arrives while the user is scrolling:
//! everything shifts down by one and the next page starts one row too late.
//! [`ListCursor`] is the sort key itself — `(received_at, id)` — so the next
//! page continues exactly where the last one ended no matter what arrived in
//! between, and the index turns it into a seek.
//!
//! [`MessageRepository::page_at`] does exist, for a scrolling `GListModel` that
//! genuinely needs random access by row number, and it carries the caveat.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, BlobId, BodyState, Disposition, EmailAddress, Flag, FlagSet, LabelId,
    LocalSyncState, MailboxId, Message, MessageId, ModSeq, OperationRange, RfcMessageId,
    ServerIdentifiers, ThreadId, Uid, UidValidity, normalize_subject,
};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params, params_from_iter};

use super::{from_millis, require_persisted, to_millis, unknown_enum};
use crate::error::{Error, Result};

/// How many rows a page holds when the caller does not say.
///
/// About two screens at the 40px row height the design canvas specifies, so a
/// scroll never waits for a query it could have started earlier.
pub const DEFAULT_PAGE_SIZE: u32 = 50;

/// What the message list shows for one message.
///
/// Deliberately not a [`Message`]: a list row carries no body, no headers, no
/// recipients beyond the sender and no attachment metadata beyond whether there
/// is one. Paging 50 of these is a few kilobytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageListRow {
    /// Local id.
    pub id: MessageId,
    /// The thread this message belongs to, so the list can group without a
    /// second query.
    pub thread_id: Option<ThreadId>,
    /// Who it is from, for the first line of the row.
    pub from: Option<EmailAddress>,
    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// The snippet under the subject.
    pub preview: Option<String>,
    /// When the server received it; the sort key.
    pub received_at: DateTime<Utc>,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it carries `\Flagged`.
    pub flagged: bool,
    /// Whether it has been replied to.
    pub answered: bool,
    /// Whether it is a draft.
    pub draft: bool,
    /// Whether it has an attachment, for the paperclip.
    pub has_attachments: bool,
    /// Size in bytes.
    pub size: u64,
}

impl MessageListRow {
    /// The cursor that resumes paging immediately after this row.
    pub fn cursor(&self) -> ListCursor {
        ListCursor {
            received_at: self.received_at,
            id: self.id,
        }
    }
}

/// A position in the list: the sort key of the last row already shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    /// The row's `received_at`.
    pub received_at: DateTime<Utc>,
    /// The row's id, which breaks ties between messages received in the same
    /// millisecond and is what makes the order total.
    pub id: MessageId,
}

/// Which messages a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListScope {
    /// One folder.
    Mailbox(MailboxId),
    /// Every folder in an account: the unified view.
    Account(AccountId),
    /// The sidebar's "Flagged" view.
    Flagged(AccountId),
    /// One conversation, wherever its messages are filed.
    ///
    /// Not a narrowing of a mailbox: a thread routinely spans folders, and
    /// the drill-in used to show only the part of it the list happened to
    /// have paged in. See `idx_messages_thread`.
    Thread(ThreadId),
}

/// One window of the message list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListQuery {
    /// Which messages.
    pub scope: ListScope,
    /// How many rows at most.
    pub limit: u32,
    /// Where to resume; `None` starts at the newest message.
    pub after: Option<ListCursor>,
}

impl ListQuery {
    /// Every message in one mailbox.
    pub fn mailbox(id: MailboxId) -> Self {
        Self::new(ListScope::Mailbox(id))
    }

    /// Every message in an account.
    pub fn account(id: AccountId) -> Self {
        Self::new(ListScope::Account(id))
    }

    /// Every flagged message in an account.
    pub fn flagged(id: AccountId) -> Self {
        Self::new(ListScope::Flagged(id))
    }

    /// Every message of one thread, in every folder it touches.
    pub fn thread(id: ThreadId) -> Self {
        Self::new(ListScope::Thread(id))
    }

    fn new(scope: ListScope) -> Self {
        Self {
            scope,
            limit: DEFAULT_PAGE_SIZE,
            after: None,
        }
    }

    /// Sets the window size.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Resumes after `cursor`.
    pub fn after(mut self, cursor: ListCursor) -> Self {
        self.after = Some(cursor);
        self
    }
}

/// A set of messages named by a predicate rather than by its members.
///
/// # Why this is not a `Vec<MessageId>`
///
/// `Ctrl+A` in an 81,717-message mailbox has to reach the statement that acts
/// on it still shaped like a *query*. `postio-core`'s
/// `Selection::Everything { except }` keeps it that way up to the handler; this
/// is where the same idea lands in SQL, so archiving a whole mailbox is one
/// `UPDATE` over an index rather than a hundred thousand ids that something had
/// to enumerate first. docs/PRODUCT.md §18 forbids the enumeration outright, and the
/// 16 ms interaction budget would not survive it either way.
///
/// Both variants render to a `WHERE` fragment, so every bulk write in this
/// crate is the same statement with a different predicate in the middle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSet {
    /// Every message a mailbox holds, less the rows taken back out of the
    /// selection.
    ///
    /// Rows hidden pending a remote delete are excluded, because the set means
    /// "what the list is showing" and the list does not show them.
    InMailbox {
        /// The folder the predicate is about.
        mailbox: MailboxId,
        /// Rows the user deselected. Built by clicking, so it is short.
        except: Vec<MessageId>,
    },
    /// Every message a run of queue rows named.
    ///
    /// This is how undo takes back a bulk action without naming its rows: the
    /// queue already wrote one row per message, in one numbered run, so the run
    /// *is* the set. See [`OperationRange`].
    Queued(OperationRange),
    /// The rows another set names whose denormalised flag column already reads
    /// `present`.
    ///
    /// This is what makes a bulk flag write *exact*. A toggle over a whole
    /// mailbox changes only the rows that disagree with it, and the queue has
    /// to carry precisely those — a queue row for a message that was already
    /// read tells the server nothing and, worse, puts that message inside the
    /// run undo takes back, so `u` would clear a flag the action never set.
    /// Narrowing is a column comparison rather than a read, so the set stays a
    /// predicate.
    WithFlag {
        /// The set being narrowed.
        set: Box<MessageSet>,
        /// The column to compare.
        flag: ColumnFlag,
        /// The value that column has to hold.
        present: bool,
    },
}

impl MessageSet {
    /// Every message a mailbox holds.
    pub fn in_mailbox(mailbox: MailboxId) -> Self {
        MessageSet::InMailbox {
            mailbox,
            except: Vec::new(),
        }
    }

    /// The folder the rows are in, when the set names one.
    ///
    /// `None` for [`MessageSet::Queued`], whose rows are wherever the operation
    /// it names put them.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            MessageSet::InMailbox { mailbox, .. } => Some(*mailbox),
            MessageSet::Queued(_) => None,
            MessageSet::WithFlag { set, .. } => set.mailbox(),
        }
    }

    /// This set narrowed to the rows whose `flag` column reads `present`.
    pub fn with_flag(self, flag: ColumnFlag, present: bool) -> Self {
        MessageSet::WithFlag {
            set: Box::new(self),
            flag,
            present,
        }
    }

    /// The `WHERE` fragment this set resolves to, and its arguments, using
    /// numbered parameters from `?first` upwards.
    ///
    /// The fragment always constrains `messages`, so it composes into any
    /// statement whose `FROM` names that table.
    pub(crate) fn predicate(&self, first: usize) -> (String, Vec<i64>) {
        match self {
            MessageSet::InMailbox { mailbox, except } => {
                let mut sql =
                    format!("messages.mailbox_id = ?{first} AND messages.deleted_locally = 0");
                if !except.is_empty() {
                    sql.push_str(&format!(
                        " AND messages.id NOT IN ({})",
                        placeholders(except.len(), first + 1)
                    ));
                }
                let mut arguments = vec![mailbox.get()];
                arguments.extend(except.iter().map(|id| id.get()));
                (sql, arguments)
            }
            // The subquery is bounded by two integers, so SQLite seeks the
            // queue's primary key rather than scanning it.
            MessageSet::Queued(range) => (
                format!(
                    "messages.id IN (SELECT target_id FROM operation_queue
                                      WHERE id BETWEEN ?{first} AND ?{}
                                        AND target_kind = 'message')",
                    first + 1
                ),
                vec![range.first.get(), range.last.get()],
            ),
            MessageSet::WithFlag { set, flag, present } => {
                let (inner, mut arguments) = set.predicate(first);
                // The inner set numbered its parameters from `first` upwards
                // and used one per argument, so the next free number is this.
                let next = first + arguments.len();
                arguments.push(i64::from(*present));
                (
                    format!("({inner}) AND messages.{} = ?{next}", flag.column()),
                    arguments,
                )
            }
        }
    }
}

/// A flag the `messages` table denormalises into a column of its own.
///
/// `\Seen` and `\Flagged` are stored twice: once inside the `flags` text and
/// once as a boolean, so the list and the sidebar filter and count without
/// parsing a string. Those two columns are what make a whole-mailbox flag
/// write affordable — the column is both the predicate that selects the rows
/// which disagree and half of the write itself, so nothing has to be read.
///
/// Everything else lives only in the text, and a bulk write over it is not
/// offered: there is no column to compare, so finding the rows that disagree
/// would mean scanning strings across the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFlag {
    /// `\Seen`, in `messages.seen`.
    Seen,
    /// `\Flagged`, in `messages.flagged`.
    Flagged,
}

impl ColumnFlag {
    /// The one this flag denormalises to, if it denormalises to one.
    pub fn of(flag: &Flag) -> Option<Self> {
        match flag {
            Flag::Seen => Some(ColumnFlag::Seen),
            Flag::Flagged => Some(ColumnFlag::Flagged),
            _ => None,
        }
    }

    /// The flag this is, in the vocabulary the rest of the application speaks.
    pub fn flag(self) -> Flag {
        match self {
            ColumnFlag::Seen => Flag::Seen,
            ColumnFlag::Flagged => Flag::Flagged,
        }
    }

    /// The column holding it.
    fn column(self) -> &'static str {
        match self {
            ColumnFlag::Seen => "seen",
            ColumnFlag::Flagged => "flagged",
        }
    }
}

/// Where a flag change came from, which decides whether the row is now ahead of
/// the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagSource {
    /// The user did it. The change has to be pushed, so the row is dirty.
    Local,
    /// The server told us. By definition not ahead of the server.
    Server,
}

/// What one [`MessageRepository::upsert_batch`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpsertReport {
    /// Messages that were not known locally.
    pub inserted: usize,
    /// Messages that already had a row under the same server identity.
    pub updated: usize,
    /// Messages this store already holds as a draft of its own, and therefore
    /// did not store a second time. See [`MessageRepository::upsert_batch`].
    pub own_drafts: usize,
    /// Messages the server still lists in a mailbox the user has already moved
    /// them out of, whose move has not reached the server yet — so re-creating
    /// the row would put back something the user watched leave. See
    /// [`MessageRepository::upsert_batch`] and #368.
    pub shadowed_by_pending: usize,
}

/// The blob keys for a message's decoded content.
///
/// The bytes live in the blob store; the row holds these. They are not on
/// [`Message`] because nothing above the storage layer should be handling blob
/// keys — it asks for the body and gets bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyBlobs {
    /// The `text/plain` body.
    pub text: Option<BlobId>,
    /// The `text/html` body.
    pub html: Option<BlobId>,
    /// The full header block.
    pub headers: Option<BlobId>,
}

/// One message still missing all or part of its body.
///
/// Everything the backfill scheduler needs to turn a row into a fetch: which
/// message, the mailbox path to ask the server for it under, and the sort key
/// it queues by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillCandidate {
    /// The local row.
    pub message_id: MessageId,
    /// The mailbox it is in.
    pub mailbox_id: MailboxId,
    /// That mailbox's path on the server, for the `FETCH` the backend issues.
    pub mailbox_path: String,
    /// The server's identifier for the message.
    pub uid: Uid,
    /// `RFC822.SIZE`, as the header fetch reported it.
    pub size: u64,
    /// When the server received it. The backlog's sort key.
    pub received_at: DateTime<Utc>,
}

/// Reads and writes [`Message`] rows.
#[derive(Debug)]
pub struct MessageRepository<'a> {
    connection: &'a Connection,
}

const MESSAGE_COLUMNS: &str = "\
id, account_id, mailbox_id, thread_id, rfc_message_id, in_reply_to, reference_ids, subject,
date, received_at, preview, size, flags, has_attachments, uid, uid_validity, mod_seq,
remote_id, body_state, flags_dirty, has_pending_operations, deleted_locally, last_synced_at,
raw_blob_id, content_type, list_id";

/// The columns a list row needs, and not one more.
///
/// The sender comes from a correlated lookup rather than a join so the plan
/// stays "walk the list index, then one index seek per row shown" — bounded by
/// the window, never by the mailbox.
/// How many ids one `IN (...)` carries.
///
/// Well under `SQLITE_MAX_VARIABLE_NUMBER`, which is 32766 on anything
/// current and 999 on builds old enough to matter. A page of hits is 50 and a
/// whole result set is capped at 200, so one chunk is the normal case — this
/// is here for the day one of those numbers moves, not for today.
const ID_CHUNK: usize = 500;

pub(crate) const LIST_COLUMNS: &str = "\
messages.id, messages.thread_id, messages.subject, messages.preview, messages.received_at,
messages.seen, messages.flagged, messages.answered, messages.draft, messages.has_attachments,
messages.size,
(SELECT name FROM recipients
  WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
  ORDER BY recipients.position LIMIT 1),
(SELECT address FROM recipients
  WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
  ORDER BY recipients.position LIMIT 1)";

impl<'a> MessageRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a message with its recipients, attachments and labels.
    ///
    /// The body and the header block are *not* written: they belong to the blob
    /// store, and their keys are set with
    /// [`MessageRepository::set_body_blobs`].
    pub fn create(&self, message: &mut Message) -> Result<MessageId> {
        let transaction = super::Scope::open(self.connection)?;
        let id = insert(&transaction, message)?;
        message.id = id;
        write_children(&transaction, message)?;
        transaction.commit()?;
        Ok(id)
    }

    /// Writes a message back, replacing its recipients, attachments and labels.
    ///
    /// Takes `&mut` because the attachment rows are rewritten and therefore
    /// re-issued: the value would otherwise be left holding ids that no longer
    /// exist.
    pub fn update(&self, message: &mut Message) -> Result<()> {
        let transaction = super::Scope::open(self.connection)?;
        write_update(&transaction, message)?;
        transaction.commit()?;
        Ok(())
    }

    /// Inserts or updates every message in `batch`, in one transaction, and
    /// removes from `batch` the ones this store already holds as drafts.
    ///
    /// This is the shape sync writes in: a `FETCH` returns a page of messages,
    /// some of which are already known. A message is "already known" when it
    /// has the same `(mailbox, UIDVALIDITY, UID)`, which is the only identity
    /// the server guarantees — a locally composed message has no UID and is
    /// therefore always an insert.
    ///
    /// One transaction rather than one per message: a resync of a thousand
    /// messages is a thousand `fsync`s otherwise, and a half-applied page is a
    /// mailbox the next sync would have to reconcile against itself.
    ///
    /// # Why it takes a `Vec` and shortens it
    ///
    /// A draft is appended to the account's Drafts mailbox, and the next pass
    /// over that folder fetches it straight back — so without this the same
    /// unfinished message exists twice locally: once as the composer's live
    /// `drafts` row, and once as a `messages` row that is a read-only snapshot
    /// of a buffer still being typed into. The composer owns a draft this
    /// client wrote (#51). A draft written by *another* client has no local
    /// draft row, is not matched here, and stays an ordinary message — which
    /// is the reason the folder is worth syncing at all.
    ///
    /// It is done here, rather than in the two sync passes that call this,
    /// because the callers go on to thread each message and record its
    /// correspondents. A skip that left the message in the batch would thread
    /// a row that was never written; a skip each caller had to remember is one
    /// a third caller would not. Shortening the `Vec` makes the batch mean
    /// "what was stored", which is what those loops already assume.
    ///
    /// # A pending move shadows what the server says (#368)
    ///
    /// The same shortening, for the same reason, on a second set. Archiving is
    /// local-first: the row moves to Archive in SQLite, a `Move` is queued, the
    /// list repaints, and the server is told when the queue drains. Until then
    /// the server still lists the message where it was, so a resync of that
    /// mailbox fetches it, finds no row under `(mailbox, validity, uid)` —
    /// because the row is in Archive now — and inserts a fresh one. The message
    /// the user just archived is back in the inbox, and stays there until the
    /// queue drains, which on a link that is down is indefinite. It then
    /// vanishes again on its own, with nothing in the interface explaining any
    /// of it.
    ///
    /// So a message with an undrained `Move` or `Delete` out of the mailbox
    /// being written is skipped: the local answer is the one the user sees
    /// until the server agrees, which is what local-first means beyond not
    /// blocking. The shadow is keyed on the queue row's *snapshot* of the
    /// server coordinates (#289), because the local half of the move nulled
    /// them on the message row itself.
    ///
    /// It lifts as soon as the operation settles — `Done` or `Failed`. A move
    /// the server refused must stop hiding the message, or a failed archive
    /// loses mail silently, which is worse than the bug this fixes.
    pub fn upsert_batch(&self, batch: &mut Vec<Message>) -> Result<UpsertReport> {
        let transaction = super::Scope::open(self.connection)?;
        let mut report = UpsertReport::default();

        // Read once for the batch rather than per message: a store holds a
        // handful of drafts, and a page holds hundreds of messages.
        let mine = own_draft_copies(&transaction)?;
        let before = batch.len();
        batch.retain(|message| {
            match (message.server.uid, message.server.uid_validity) {
                (Some(uid), Some(validity)) => {
                    !mine.contains(&(message.mailbox_id, validity.get(), uid.get()))
                }
                // Nowhere to have landed, so nothing to be a second copy of.
                _ => true,
            }
        });
        report.own_drafts = before - batch.len();

        // The same shape again, on the messages the user has already moved
        // out of this mailbox and the server has not been told about yet.
        let shadowed = shadowed_by_pending_operation(&transaction)?;
        let before = batch.len();
        batch.retain(|message| {
            match (message.server.uid, message.server.uid_validity) {
                (Some(uid), Some(validity)) => {
                    !shadowed.contains(&(message.mailbox_id, validity.get(), uid.get()))
                }
                // No server identity, so no queue row can be shadowing it: the
                // snapshot is of server coordinates, and this message has none.
                _ => true,
            }
        });
        report.shadowed_by_pending = before - batch.len();

        for message in batch.iter_mut() {
            let existing = match (message.server.uid, message.server.uid_validity) {
                (Some(uid), Some(validity)) => {
                    find_by_uid(&transaction, message.mailbox_id, validity, uid)?
                }
                _ => None,
            };

            match existing {
                Some(id) => {
                    message.id = id;
                    write_update(&transaction, message)?;
                    report.updated += 1;
                }
                None => {
                    message.id = insert(&transaction, message)?;
                    write_children(&transaction, message)?;
                    report.inserted += 1;
                }
            }
        }

        transaction.commit()?;
        Ok(report)
    }

    /// One message, with its recipients, attachments and labels.
    ///
    /// The body and headers are empty: those bytes are in the blob store. See
    /// [`MessageRepository::body_blobs`].
    pub fn get(&self, id: MessageId) -> Result<Option<Message>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mut message = read_message(row)?;
        drop(rows);
        drop(statement);

        read_recipients(self.connection, &mut message)?;
        message.attachments = read_attachments(self.connection, id)?;
        message.labels = read_labels(self.connection, id)?;
        Ok(Some(message))
    }

    /// One window of the message list, newest first.
    pub fn page(&self, query: &ListQuery) -> Result<Vec<MessageListRow>> {
        let sql = self.explain(query);
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(page_arguments(query)), read_list_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One window at a row offset, for a list model that scrolls by index.
    ///
    /// Prefer [`MessageRepository::page`]: an offset is counted from the top of
    /// the list every time, so a message arriving while the user scrolls shifts
    /// every row down and this window silently skips one.
    pub fn page_at(&self, query: &ListQuery, offset: u32) -> Result<Vec<MessageListRow>> {
        let sql = format!("{} OFFSET {offset}", self.explain(query));
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(page_arguments(query)), read_list_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The list rows for `ids`, in the order given.
    ///
    /// For a set of search hits, which none of the [`ListScope`]s fits: they
    /// are ranked rather than sorted, they can span folders, and there is no
    /// offset to page by because the ids themselves are the answer.
    ///
    /// **The order is restored after the read, and that is the point.** SQL
    /// answers an `IN` in whatever order it walks the rows, which for the
    /// `messages` primary key is id order — near enough to date order that a
    /// lost ranking would look right in every fixture and be wrong exactly
    /// where relevance was what the user wanted.
    ///
    /// Ids the store no longer holds are dropped rather than faked. A message
    /// deleted between the search and this read is a shorter answer, not an
    /// error — the index and the store are allowed to disagree for a moment,
    /// and the list can render 199 rows perfectly well.
    pub fn rows_for(&self, ids: &[MessageId]) -> Result<Vec<MessageListRow>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut found: std::collections::HashMap<MessageId, MessageListRow> =
            std::collections::HashMap::with_capacity(ids.len());
        for chunk in ids.chunks(ID_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT {LIST_COLUMNS} FROM messages WHERE messages.id IN ({placeholders})"
            );
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(
                params_from_iter(chunk.iter().map(|id| id.get())),
                read_list_row,
            )?;
            for row in rows {
                let row = row?;
                found.insert(row.id, row);
            }
        }

        Ok(ids.iter().filter_map(|id| found.get(id).cloned()).collect())
    }

    /// How many messages the list would show. Ignores the window.
    pub fn count(&self, query: &ListQuery) -> Result<u32> {
        let sql = format!(
            "SELECT count(*) FROM messages WHERE {}",
            where_clause(query, false)
        );
        let mut arguments = scope_arguments(&query.scope);
        let count: i64 =
            self.connection
                .query_row(&sql, params_from_iter(arguments.drain(..)), |row| {
                    row.get(0)
                })?;
        Ok(count as u32)
    }

    /// The SQL a page query runs.
    ///
    /// Exposed so a test can put `EXPLAIN QUERY PLAN` in front of it: the list
    /// query is on the hot path, and "does this still use the index and still
    /// avoid a sort" is a property worth asserting rather than remembering.
    pub fn explain(&self, query: &ListQuery) -> String {
        format!(
            "SELECT {LIST_COLUMNS} FROM messages WHERE {} \
             ORDER BY messages.received_at DESC, messages.id DESC LIMIT {}",
            where_clause(query, query.after.is_some()),
            query.limit
        )
    }

    /// Replaces a message's flags.
    pub fn set_flags(&self, id: MessageId, flags: &FlagSet, source: FlagSource) -> Result<()> {
        let flags = flags.persistable();
        let changed = self.connection.execute(
            "UPDATE messages
                SET flags = ?2, seen = ?3, flagged = ?4, answered = ?5, draft = ?6,
                    deleted = ?7, flags_dirty = ?8
              WHERE id = ?1",
            params![
                id.get(),
                flag_text(&flags),
                flags.is_seen(),
                flags.is_flagged(),
                flags.is_answered(),
                flags.is_draft(),
                flags.is_deleted(),
                source == FlagSource::Local,
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Moves messages into another mailbox, returning how many moved.
    ///
    /// The server identity is cleared: a UID belongs to the mailbox that issued
    /// it, and keeping it would make the next resync match the wrong message.
    /// The rows are marked as having pending operations, because the move still
    /// has to reach the server.
    pub fn move_to(&self, ids: &[MessageId], mailbox_id: MailboxId) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages
                SET mailbox_id = ?1, uid = NULL, uid_validity = NULL, mod_seq = NULL,
                    has_pending_operations = 1
              WHERE id IN ({})",
            placeholders(ids.len(), 2)
        );
        let mut arguments = vec![mailbox_id.get()];
        arguments.extend(ids.iter().map(|id| id.get()));
        Ok(self.connection.execute(&sql, params_from_iter(arguments))?)
    }

    /// Moves every message a [`MessageSet`] names into `mailbox_id`,
    /// returning how many moved.
    ///
    /// The bulk twin of [`MessageRepository::move_to`], and the reason
    /// `Ctrl+A` then `a` is affordable: one `UPDATE` over the mailbox index,
    /// whatever the mailbox holds. Nothing is read first — not to count the
    /// rows, not to name them — so the cost is the write and no more.
    ///
    /// The server identity is cleared for the same reason [`move_to`] clears
    /// it: a UID belongs to the mailbox that issued it.
    ///
    /// [`move_to`]: MessageRepository::move_to
    pub fn move_set(&self, set: &MessageSet, mailbox_id: MailboxId) -> Result<usize> {
        let (predicate, arguments) = set.predicate(2);
        let sql = format!(
            "UPDATE messages
                SET mailbox_id = ?1, uid = NULL, uid_validity = NULL, mod_seq = NULL,
                    has_pending_operations = 1
              WHERE {predicate}"
        );
        let mut parameters = vec![mailbox_id.get()];
        parameters.extend(arguments);
        Ok(self
            .connection
            .execute(&sql, params_from_iter(parameters))?)
    }

    /// How many messages a [`MessageSet`] names.
    ///
    /// One indexed `count(*)`. The undo toast needs a number — *Archived
    /// 81,717 messages* — and this is the only thing about a bulk action that
    /// has to know one.
    pub fn count_set(&self, set: &MessageSet) -> Result<u32> {
        let (predicate, arguments) = set.predicate(1);
        let sql = format!("SELECT count(*) FROM messages WHERE {predicate}");
        let count: i64 = self
            .connection
            .query_row(&sql, params_from_iter(arguments), |row| row.get(0))?;
        Ok(count as u32)
    }

    /// Sets or clears one flag across every message a [`MessageSet`] names,
    /// returning how many rows it wrote.
    ///
    /// The bulk twin of [`set_flags`], and the second half of what makes
    /// `Ctrl+A` affordable: one `UPDATE` over the mailbox index, whatever the
    /// mailbox holds. Nothing is read — not the rows, not their flags.
    ///
    /// # Why the text can be rebuilt without reading it
    ///
    /// `messages.flags` is documented as canonical spellings in [`FlagSet`]
    /// order, and the five system flags with columns of their own — `\Seen`,
    /// `\Answered`, `\Flagged`, `\Deleted`, `\Draft` — are exactly the five
    /// lowest-ranked persistable flags. So the text is always those five, in
    /// column order, followed by whatever keywords the row also carries. The
    /// statement below rebuilds the head from the booleans (substituting the
    /// value it is writing for the one being changed) and keeps the tail by
    /// stripping the five system spellings out of the text it already has.
    /// Appending instead would be one `replace` shorter and would put
    /// `\Seen` last, quietly breaking the ordering the schema promises.
    ///
    /// Rows that already agree are still matched unless the caller narrowed
    /// the set with [`MessageSet::with_flag`] — this writes what it is told
    /// to. It is the caller that has to care, because the queue rows and the
    /// undo entry have to cover the same rows this does.
    ///
    /// [`set_flags`]: MessageRepository::set_flags
    pub fn set_flag_on_set(
        &self,
        set: &MessageSet,
        flag: ColumnFlag,
        present: bool,
    ) -> Result<usize> {
        // `SET` reads the row as it was, so the column being written still
        // holds the old value here and the new one has to be spliced in.
        let (seen, flagged) = match flag {
            ColumnFlag::Seen => ("?1", "messages.flagged"),
            ColumnFlag::Flagged => ("messages.seen", "?1"),
        };
        let (predicate, arguments) = set.predicate(2);
        let sql = format!(
            "UPDATE messages
                SET flags = {}, {} = ?1, flags_dirty = 1
              WHERE {predicate}",
            flags_expression(seen, flagged),
            flag.column(),
        );
        let mut parameters = vec![i64::from(present)];
        parameters.extend(arguments);
        Ok(self
            .connection
            .execute(&sql, params_from_iter(parameters))?)
    }

    /// Hides messages pending a remote delete or move, or brings them back.
    ///
    /// This is what makes delete feel instant and undo possible: the row stays,
    /// the list stops showing it.
    pub fn set_deleted_locally(&self, ids: &[MessageId], deleted: bool) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages SET deleted_locally = ?1, has_pending_operations = ?1
              WHERE id IN ({})",
            placeholders(ids.len(), 2)
        );
        let mut arguments = vec![i64::from(deleted)];
        arguments.extend(ids.iter().map(|id| id.get()));
        Ok(self.connection.execute(&sql, params_from_iter(arguments))?)
    }

    /// Removes messages outright, returning how many there were.
    ///
    /// For an expunge the server has confirmed. Recipients, attachments and
    /// label links cascade; the blobs are swept separately.
    pub fn delete(&self, ids: &[MessageId]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "DELETE FROM messages WHERE id IN ({})",
            placeholders(ids.len(), 1)
        );
        Ok(self
            .connection
            .execute(&sql, params_from_iter(ids.iter().map(|id| id.get())))?)
    }

    /// Puts a message in a thread, or takes it out of one.
    pub fn set_thread(&self, id: MessageId, thread_id: Option<ThreadId>) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            params![id.get(), thread_id.map(ThreadId::get)],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// The message with this server identity, if it is known locally.
    pub fn by_uid(
        &self,
        mailbox_id: MailboxId,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Option<Message>> {
        match find_by_uid(self.connection, mailbox_id, uid_validity, uid)? {
            Some(id) => self.get(id),
            None => Ok(None),
        }
    }

    /// Every UID known locally for a mailbox under `uid_validity`, ascending.
    ///
    /// What a resync diffs the server's UID list against.
    pub fn uids_in(&self, mailbox_id: MailboxId, uid_validity: UidValidity) -> Result<Vec<Uid>> {
        let mut statement = self.connection.prepare(
            "SELECT uid FROM messages
              WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid IS NOT NULL
              ORDER BY uid",
        )?;
        let rows = statement.query_map(
            params![mailbox_id.get(), i64::from(uid_validity.get())],
            |row| Ok(Uid::new(row.get::<_, i64>(0)? as u32)),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every message in an account carrying this `Message-ID`.
    ///
    /// A list, not an `Option`: `Message-ID` is not unique in the wild. Threads
    /// have to cope with two messages claiming the same one, and so does
    /// deduplication.
    pub fn ids_by_rfc_message_id(
        &self,
        account_id: AccountId,
        rfc_message_id: &RfcMessageId,
    ) -> Result<Vec<MessageId>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM messages
              WHERE account_id = ?1 AND rfc_message_id = ?2 COLLATE NOCASE
              ORDER BY id",
        )?;
        let rows = statement
            .query_map(params![account_id.get(), rfc_message_id.as_str()], |row| {
                Ok(MessageId::new(row.get(0)?))
            })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The blob keys for a message's decoded content.
    pub fn body_blobs(&self, id: MessageId) -> Result<Option<BodyBlobs>> {
        let mut statement = self.connection.prepare(
            "SELECT body_text_blob_id, body_html_blob_id, headers_blob_id
               FROM messages WHERE id = ?1",
        )?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(BodyBlobs {
            text: row.get::<_, Option<String>>(0)?.map(BlobId::new),
            html: row.get::<_, Option<String>>(1)?.map(BlobId::new),
            headers: row.get::<_, Option<String>>(2)?.map(BlobId::new),
        }))
    }

    /// Records where a message's content landed in the blob store.
    ///
    /// Takes the new [`BodyState`] with it: the keys and "how much of this
    /// message is local" are one fact, and writing them separately would leave
    /// a window where the backfill queue disagrees with the reader.
    pub fn set_body_blobs(
        &self,
        id: MessageId,
        blobs: &BodyBlobs,
        body_state: BodyState,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE messages
                SET body_text_blob_id = ?2, body_html_blob_id = ?3, headers_blob_id = ?4,
                    body_state = ?5
              WHERE id = ?1",
            params![
                id.get(),
                blobs.text.as_ref().map(BlobId::as_str),
                blobs.html.as_ref().map(BlobId::as_str),
                blobs.headers.as_ref().map(BlobId::as_str),
                body_state.as_str(),
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Messages in `mailbox_id` still missing all or part of their body,
    /// newest first and windowed to `limit` — what a cold start, or a resync
    /// that just finished, seeds the backfill scheduler with.
    ///
    /// Scoped to one mailbox because that is how the caller always has this
    /// question: `postio-sync`'s initial and incremental syncs are already
    /// mailbox at a time, and `idx_messages_body_state` is a partial index on
    /// exactly `(mailbox_id, received_at DESC) WHERE body_state <> 'full'`, so
    /// this is a seek, never a scan.
    ///
    /// A message with no `UID` yet is not a candidate: there is nothing to
    /// issue a `FETCH` against until the server has assigned one.
    pub fn needing_backfill(
        &self,
        mailbox_id: MailboxId,
        limit: u32,
    ) -> Result<Vec<BackfillCandidate>> {
        let mut statement = self.connection.prepare(
            "SELECT messages.id, messages.uid, messages.size, messages.received_at,
                    mailboxes.path
               FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
              WHERE messages.mailbox_id = ?1
                AND messages.body_state <> 'full'
                AND messages.uid IS NOT NULL
                AND messages.deleted_locally = 0
              ORDER BY messages.received_at DESC
              LIMIT ?2",
        )?;
        let rows = statement.query_map(params![mailbox_id.get(), limit], |row| {
            read_backfill_candidate(row, mailbox_id)
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One message's backfill candidate, if it still needs (part of) its body.
    ///
    /// For the interactive lane: the reading pane knows only which message was
    /// opened, not which mailbox it lives in or what the server calls it, so
    /// this looks both up rather than asking the caller to already know them.
    /// `None` covers both "already has a full body" and "not there any more" —
    /// either way there is nothing to fetch.
    pub fn backfill_candidate(&self, message_id: MessageId) -> Result<Option<BackfillCandidate>> {
        let mut statement = self.connection.prepare(
            "SELECT messages.id, messages.uid, messages.size, messages.received_at,
                    mailboxes.path, messages.mailbox_id
               FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
              WHERE messages.id = ?1
                AND messages.body_state <> 'full'
                AND messages.uid IS NOT NULL
                AND messages.deleted_locally = 0",
        )?;
        let mut rows = statement.query([message_id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mailbox_id = MailboxId::new(row.get(5)?);
        Ok(Some(read_backfill_candidate(row, mailbox_id)?))
    }

    /// Messages in `mailbox_id` that carry no usable thread reference at all —
    /// no `In-Reply-To` and an empty `References` — and are still threaded.
    ///
    /// These are the only messages [`crate::repository::ThreadingRepository::reconsider`]
    /// can ever have a better answer for later than it did at insertion: one
    /// that names an ancestor either already found it or is waiting to, so
    /// only silence at insertion time is worth asking about again.
    pub fn subject_only_orphans(&self, mailbox_id: MailboxId) -> Result<Vec<MessageId>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM messages
              WHERE mailbox_id = ?1
                AND thread_id IS NOT NULL
                AND in_reply_to IS NULL
                AND reference_ids = ''",
        )?;
        let rows =
            statement.query_map([mailbox_id.get()], |row| Ok(MessageId::new(row.get(0)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

// ---------------------------------------------------------------------------
// SQL construction
// ---------------------------------------------------------------------------

/// The `WHERE` of a list query.
///
/// `deleted_locally = 0` is here rather than in the index because it is a
/// filter on a handful of rows, not a way in: the list index gives the order
/// and the scope gives the range.
fn where_clause(query: &ListQuery, with_cursor: bool) -> String {
    let scope = match query.scope {
        ListScope::Mailbox(_) => "messages.mailbox_id = ?1",
        ListScope::Account(_) => "messages.account_id = ?1",
        ListScope::Flagged(_) => "messages.account_id = ?1 AND messages.flagged = 1",
        ListScope::Thread(_) => "messages.thread_id = ?1",
    };
    let cursor = if with_cursor {
        // A row value, so SQLite can turn it into one range constraint on
        // (received_at, id) and seek straight to the cursor. Spelled as an OR
        // it would be a filter, and a deep page would walk every row above it.
        " AND (messages.received_at, messages.id) < (?2, ?3)"
    } else {
        ""
    };
    format!("{scope} AND messages.deleted_locally = 0{cursor}")
}

fn scope_arguments(scope: &ListScope) -> Vec<i64> {
    vec![match scope {
        ListScope::Mailbox(id) => id.get(),
        ListScope::Account(id) | ListScope::Flagged(id) => id.get(),
        ListScope::Thread(id) => id.get(),
    }]
}

fn page_arguments(query: &ListQuery) -> Vec<i64> {
    let mut arguments = scope_arguments(&query.scope);
    if let Some(cursor) = query.after {
        arguments.push(to_millis(cursor.received_at));
        arguments.push(cursor.id.get());
    }
    arguments
}

/// `?n, ?n+1, ...` for `count` parameters starting at `first`.
pub(crate) fn placeholders(count: usize, first: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", index + first))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------

/// The body of an update, without a transaction of its own.
///
/// Shared with [`MessageRepository::upsert_batch`], which is already inside
/// one: SQLite has no nested transactions, and a resync must not commit half a
/// page of messages.
fn write_update(connection: &Connection, message: &mut Message) -> Result<()> {
    let id = require_persisted(message.id.get(), "message")?;

    let changed = connection.execute(
        "UPDATE messages
            SET account_id = ?2, mailbox_id = ?3, thread_id = ?4, rfc_message_id = ?5,
                in_reply_to = ?6, reference_ids = ?7, subject = ?8,
                normalized_subject = ?9, date = ?10, received_at = ?11, preview = ?12,
                size = ?13, flags = ?14, seen = ?15, flagged = ?16, answered = ?17,
                draft = ?18, deleted = ?19, has_attachments = ?20, uid = ?21,
                uid_validity = ?22, mod_seq = ?23, remote_id = ?24, body_state = ?25,
                flags_dirty = ?26, has_pending_operations = ?27, deleted_locally = ?28,
                last_synced_at = ?29, raw_blob_id = ?30, content_type = ?31, list_id = ?32
          WHERE id = ?1",
        params_from_iter(row_values(id, message)),
    )?;
    if changed == 0 {
        return Err(Error::NotFound {
            entity: "message",
            id,
        });
    }

    connection.execute("DELETE FROM recipients WHERE message_id = ?1", [id])?;
    connection.execute("DELETE FROM attachments WHERE message_id = ?1", [id])?;
    connection.execute("DELETE FROM message_labels WHERE message_id = ?1", [id])?;
    write_children(connection, message)?;
    Ok(())
}

fn insert(connection: &Connection, message: &Message) -> Result<MessageId> {
    connection.execute(
        "INSERT INTO messages (id, account_id, mailbox_id, thread_id, rfc_message_id,
                               in_reply_to, reference_ids, subject, normalized_subject, date,
                               received_at, preview, size, flags, seen, flagged, answered,
                               draft, deleted, has_attachments, uid, uid_validity, mod_seq,
                               remote_id, body_state, flags_dirty, has_pending_operations,
                               deleted_locally, last_synced_at, raw_blob_id, content_type,
                               list_id)
         VALUES (NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                 ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31,
                 ?32)",
        params_from_iter(row_values(0, message)),
    )?;
    Ok(MessageId::new(connection.last_insert_rowid()))
}

/// The parameter list shared by insert and update, `?1` being the id.
///
/// Owned [`Value`]s rather than borrows: the two statements have thirty
/// parameters of six different types, and one homogeneous list is far easier to
/// keep aligned with the column list than two hand-written `params!` calls that
/// have to stay in step with each other.
fn row_values(id: i64, message: &Message) -> Vec<Value> {
    let flags = message.flags.persistable();
    let references = message
        .references
        .iter()
        .map(RfcMessageId::as_str)
        .collect::<Vec<_>>()
        .join(" ");

    vec![
        integer(id),
        integer(message.account_id.get()),
        integer(message.mailbox_id.get()),
        maybe_integer(message.thread_id.map(ThreadId::get)),
        maybe_text(
            message
                .rfc_message_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
        ),
        maybe_text(
            message
                .in_reply_to
                .as_ref()
                .map(|id| id.as_str().to_owned()),
        ),
        text(references),
        maybe_text(message.subject.clone()),
        maybe_text(message.subject.as_deref().map(normalize_subject)),
        maybe_integer(message.date.map(to_millis)),
        integer(to_millis(message.received_at)),
        maybe_text(message.preview.clone()),
        integer(message.size as i64),
        text(flag_text(&flags)),
        boolean(flags.is_seen()),
        boolean(flags.is_flagged()),
        boolean(flags.is_answered()),
        boolean(flags.is_draft()),
        boolean(flags.is_deleted()),
        boolean(message.has_attachments()),
        maybe_integer(message.server.uid.map(|uid| i64::from(uid.get()))),
        maybe_integer(
            message
                .server
                .uid_validity
                .map(|validity| i64::from(validity.get())),
        ),
        maybe_integer(message.server.mod_seq.map(|seq| seq.get() as i64)),
        maybe_text(message.server.remote_id.clone()),
        text(message.sync.body_state.as_str()),
        boolean(message.sync.flags_dirty),
        boolean(message.sync.has_pending_operations),
        boolean(message.sync.deleted_locally),
        maybe_integer(message.sync.last_synced_at.map(to_millis)),
        maybe_text(
            message
                .raw_blob_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
        ),
        maybe_text(message.content_type.clone()),
        maybe_text(message.list_id.clone()),
    ]
}

fn integer(value: i64) -> Value {
    Value::Integer(value)
}

fn maybe_integer(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Integer)
}

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

fn maybe_text(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::Text)
}

fn boolean(value: bool) -> Value {
    Value::Integer(i64::from(value))
}

fn write_children(connection: &Connection, message: &mut Message) -> Result<()> {
    let id = message.id.get();

    for (kind, addresses) in [
        ("from", message.from.clone()),
        ("sender", message.sender.clone().into_iter().collect()),
        ("reply_to", message.reply_to.clone()),
        ("to", message.to.clone()),
        ("cc", message.cc.clone()),
        ("bcc", message.bcc.clone()),
    ] {
        for (position, address) in addresses.iter().enumerate() {
            connection.execute(
                "INSERT INTO recipients (message_id, kind, position, name, address,
                                         address_normalized)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    kind,
                    position as i64,
                    address.name,
                    address.address,
                    address.normalized(),
                ],
            )?;
        }
    }

    for (position, attachment) in message.attachments.iter_mut().enumerate() {
        connection.execute(
            "INSERT INTO attachments (message_id, position, filename, mime_type, size,
                                      content_id, disposition, disposition_raw, part_id,
                                      blob_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                position as i64,
                attachment.filename,
                attachment.mime_type,
                attachment.size as i64,
                attachment.content_id,
                attachment.disposition.as_str(),
                attachment.disposition.raw(),
                attachment.part_id,
                attachment.blob_id.as_ref().map(BlobId::as_str),
            ],
        )?;
        attachment.id = postio_model::AttachmentId::new(connection.last_insert_rowid());
        attachment.message_id = message.id;
    }

    for label in &message.labels {
        connection.execute(
            "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            params![id, label.get()],
        )?;
    }

    Ok(())
}

fn find_by_uid(
    connection: &Connection,
    mailbox_id: MailboxId,
    uid_validity: UidValidity,
    uid: Uid,
) -> Result<Option<MessageId>> {
    let mut statement = connection.prepare(
        "SELECT id FROM messages
          WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid = ?3",
    )?;
    let mut rows = statement.query(params![
        mailbox_id.get(),
        i64::from(uid_validity.get()),
        i64::from(uid.get()),
    ])?;
    Ok(match rows.next()? {
        Some(row) => Some(MessageId::new(row.get(0)?)),
        None => None,
    })
}

/// Reads a [`BackfillCandidate`] from a row of `(id, uid, size, received_at,
/// mailbox_path)`, in that order — the shared column shape of
/// [`MessageRepository::needing_backfill`] and
/// [`MessageRepository::backfill_candidate`].
fn read_backfill_candidate(
    row: &Row<'_>,
    mailbox_id: MailboxId,
) -> rusqlite::Result<BackfillCandidate> {
    Ok(BackfillCandidate {
        message_id: MessageId::new(row.get(0)?),
        mailbox_id,
        uid: Uid::new(row.get::<_, i64>(1)? as u32),
        size: row.get::<_, i64>(2)? as u64,
        received_at: from_millis(row.get(3)?),
        mailbox_path: row.get(4)?,
    })
}

fn read_message(row: &Row<'_>) -> rusqlite::Result<Message> {
    let body_state: String = row.get(18)?;
    let references: String = row.get(6)?;

    Ok(Message {
        id: MessageId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        mailbox_id: MailboxId::new(row.get(2)?),
        thread_id: row.get::<_, Option<i64>>(3)?.map(ThreadId::new),
        rfc_message_id: row.get::<_, Option<String>>(4)?.map(RfcMessageId::new),
        in_reply_to: row.get::<_, Option<String>>(5)?.map(RfcMessageId::new),
        references: references
            .split_whitespace()
            .map(RfcMessageId::new)
            .collect(),
        from: Vec::new(),
        sender: None,
        reply_to: Vec::new(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: row.get(7)?,
        date: row.get::<_, Option<i64>>(8)?.map(from_millis),
        received_at: from_millis(row.get(9)?),
        body: postio_model::MessageBody::default(),
        preview: row.get(10)?,
        attachments: Vec::new(),
        flags: parse_flags(&row.get::<_, String>(12)?),
        labels: Vec::new(),
        size: row.get::<_, i64>(11)? as u64,
        headers: postio_model::Headers::new(),
        server: ServerIdentifiers {
            uid: row
                .get::<_, Option<i64>>(14)?
                .map(|uid| Uid::new(uid as u32)),
            uid_validity: row
                .get::<_, Option<i64>>(15)?
                .map(|validity| UidValidity::new(validity as u32)),
            mod_seq: row
                .get::<_, Option<i64>>(16)?
                .map(|seq| ModSeq::new(seq as u64)),
            remote_id: row.get(17)?,
        },
        sync: LocalSyncState {
            body_state: BodyState::from_name(&body_state).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    18,
                    rusqlite::types::Type::Text,
                    Box::new(unknown_enum("messages.body_state", body_state)),
                )
            })?,
            flags_dirty: row.get(19)?,
            has_pending_operations: row.get(20)?,
            deleted_locally: row.get(21)?,
            last_synced_at: row.get::<_, Option<i64>>(22)?.map(from_millis),
        },
        raw_blob_id: row.get::<_, Option<String>>(23)?.map(BlobId::new),
        content_type: row.get(24)?,
        list_id: row.get(25)?,
    })
}

pub(crate) fn read_list_row(row: &Row<'_>) -> rusqlite::Result<MessageListRow> {
    let from_address: Option<String> = row.get(12)?;
    Ok(MessageListRow {
        id: MessageId::new(row.get(0)?),
        thread_id: row.get::<_, Option<i64>>(1)?.map(ThreadId::new),
        from: from_address
            .map(|address| {
                Ok::<_, rusqlite::Error>(EmailAddress::new(
                    row.get::<_, Option<String>>(11)?,
                    address,
                ))
            })
            .transpose()?,
        subject: row.get(2)?,
        preview: row.get(3)?,
        received_at: from_millis(row.get(4)?),
        seen: row.get(5)?,
        flagged: row.get(6)?,
        answered: row.get(7)?,
        draft: row.get(8)?,
        has_attachments: row.get(9)?,
        size: row.get::<_, i64>(10)? as u64,
    })
}

fn read_recipients(connection: &Connection, message: &mut Message) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT kind, name, address FROM recipients
          WHERE message_id = ?1 ORDER BY kind, position, id",
    )?;
    let rows = statement.query_map([message.id.get()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            EmailAddress::new(row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?),
        ))
    })?;

    for row in rows {
        let (kind, address) = row?;
        match kind.as_str() {
            "from" => message.from.push(address),
            "sender" => message.sender = Some(address),
            "reply_to" => message.reply_to.push(address),
            "to" => message.to.push(address),
            "cc" => message.cc.push(address),
            "bcc" => message.bcc.push(address),
            other => return Err(unknown_enum("recipients.kind", other)),
        }
    }
    Ok(())
}

fn read_attachments(connection: &Connection, id: MessageId) -> Result<Vec<Attachment>> {
    let mut statement = connection.prepare(
        "SELECT id, filename, mime_type, size, content_id, disposition, disposition_raw,
                part_id, blob_id
           FROM attachments WHERE message_id = ?1 ORDER BY position, id",
    )?;
    let rows = statement.query_map([id.get()], |row| {
        let disposition: String = row.get(5)?;
        let raw: Option<String> = row.get(6)?;
        Ok(Attachment {
            id: postio_model::AttachmentId::new(row.get(0)?),
            message_id: id,
            filename: row.get(1)?,
            mime_type: row.get(2)?,
            size: row.get::<_, i64>(3)? as u64,
            content_id: row.get(4)?,
            disposition: Disposition::from_parts(&disposition, raw.as_deref()).ok_or_else(
                || {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(unknown_enum("attachments.disposition", disposition)),
                    )
                },
            )?,
            part_id: row.get(7)?,
            blob_id: row.get::<_, Option<String>>(8)?.map(BlobId::new),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn read_labels(connection: &Connection, id: MessageId) -> Result<Vec<LabelId>> {
    let mut statement = connection
        .prepare("SELECT label_id FROM message_labels WHERE message_id = ?1 ORDER BY label_id")?;
    let rows = statement.query_map([id.get()], |row| Ok(LabelId::new(row.get(0)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Flags as the schema stores them: canonical spellings, space separated, in
/// [`FlagSet`] order, with `\Recent` already gone.
/// SQL rebuilding a row's `flags` text out of its own columns.
///
/// `seen` and `flagged` are given as SQL expressions rather than read from the
/// row, so a bulk write can substitute the value it is setting for the one the
/// column still holds. See [`MessageRepository::set_flag_on_set`] for why this
/// is exact.
fn flags_expression(seen: &str, flagged: &str) -> String {
    // The five system flags, in the order `FlagSet` keeps them.
    let system = [
        (Flag::Seen.as_str(), seen.to_owned()),
        (Flag::Answered.as_str(), "messages.answered".to_owned()),
        (Flag::Flagged.as_str(), flagged.to_owned()),
        (Flag::Deleted.as_str(), "messages.deleted".to_owned()),
        (Flag::Draft.as_str(), "messages.draft".to_owned()),
    ];
    // Whatever is left once those five are out of the text: the keywords, in
    // the order they were already in, which is the order they belong in.
    let mut rest = "' ' || messages.flags || ' '".to_owned();
    for (spelling, _) in &system {
        rest = format!("replace({rest}, ' {spelling} ', ' ')");
    }
    let head = system
        .iter()
        .map(|(spelling, column)| format!("iif({column}, '{spelling} ', '')"))
        .collect::<Vec<_>>()
        .join(" || ");
    format!("trim({head} || ltrim({rest}))")
}

/// Where every draft this client has uploaded is sitting on the server.
///
/// `(mailbox, UIDVALIDITY, UID)` — the same identity `find_by_uid` matches on,
/// because that is the only one the server guarantees. Scoped to the account's
/// Drafts mailbox: UIDs are per-mailbox, so the message that happens to be
/// number 7 in the inbox has nothing to do with the draft that is number 7 in
/// Drafts. A draft whose append the server would not locate has no `uid` and
/// is absent from this — `postio-sync` flags the folder for a resync instead
/// of guessing which message is the one it just wrote.
fn own_draft_copies(connection: &Connection) -> Result<BTreeSet<(MailboxId, u32, u32)>> {
    let mut statement = connection.prepare(
        "SELECT mailboxes.id, drafts.uid_validity, drafts.uid
           FROM drafts
           JOIN mailboxes ON mailboxes.account_id = drafts.account_id
                         AND mailboxes.role = 'drafts'
          WHERE drafts.uid IS NOT NULL AND drafts.uid_validity IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            MailboxId::new(row.get::<_, i64>(0)?),
            row.get::<_, i64>(1)? as u32,
            row.get::<_, i64>(2)? as u32,
        ))
    })?;
    rows.collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(Into::into)
}

/// `(mailbox, uid_validity, uid)` for every message with an undrained
/// operation moving it *out* of that mailbox — see
/// [`MessageRepository::upsert_batch`] and #368.
///
/// Only `move` and `delete` qualify: they are the operations whose queue row
/// names the mailbox the message is leaving (`Operation::mailbox()` returns
/// `from` for both). A flag change does not move anything, and an `append`
/// puts a message *into* a mailbox, so neither can make a fetched row a
/// resurrection.
///
/// Only `pending` and `in_flight` qualify. `done` means the server has agreed
/// and will stop listing the message where it was; `failed` means the move is
/// not going to happen, and a shadow that outlived it would hide the message
/// for ever — which is a worse bug than the one this prevents.
///
/// Keyed on the queue row's snapshot rather than the message row, because the
/// local half of the move nulls the row's `uid`/`uid_validity` in the same
/// transaction that enqueues (#289): by the time a resync runs, this row is
/// the only thing that still remembers where the server has it.
fn shadowed_by_pending_operation(
    connection: &Connection,
) -> Result<BTreeSet<(MailboxId, u32, u32)>> {
    let mut statement = connection.prepare(
        "SELECT mailbox_id, source_uid_validity, source_uid
           FROM operation_queue
          WHERE state IN ('pending', 'in_flight')
            AND op_type IN ('move', 'delete')
            AND mailbox_id IS NOT NULL
            AND source_uid IS NOT NULL
            AND source_uid_validity IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            MailboxId::new(row.get::<_, i64>(0)?),
            row.get::<_, i64>(1)? as u32,
            row.get::<_, i64>(2)? as u32,
        ))
    })?;
    rows.collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(Into::into)
}

fn flag_text(flags: &FlagSet) -> String {
    flags.iter().map(Flag::as_str).collect::<Vec<_>>().join(" ")
}

fn parse_flags(text: &str) -> FlagSet {
    text.split_whitespace().map(Flag::parse).collect()
}
