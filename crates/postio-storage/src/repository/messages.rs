//! Messages, and the windowed query the message list pages through. The only way to read more than
//! one message is [`MessageRepository::page`], at most [`ListQuery::limit`] rows of list-row
//! fields; there is deliberately no `all()`.
//! Cursor, not offset: [`ListCursor`] is the sort key itself, so a page continues where the last
//! ended whatever arrived meanwhile, and the index turns it into a seek.
//! [`MessageRepository::page_at`] exists for random access by row number and carries the caveat.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, BlobId, BodyState, Disposition, DraftState, EmailAddress, Flag, FlagSet,
    Generation, LabelId, LocalSyncState, MailboxId, MailboxRole, Message, MessageId, ModSeq,
    OperationRange, RemoteId, RfcMessageId, ServerIdentifiers, ThreadId, Uid, UidValidity,
    normalize_subject,
};

pub use postio_model::ListScope;

use postio_model::promoted::PromotedHeaders;

use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

pub const DEFAULT_PAGE_SIZE: u32 = 50;

/// Not a [`Message`]: no body, headers, recipients beyond the sender, or attachment metadata beyond
/// whether there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageListRow {
    pub id: MessageId,
    pub thread_id: Option<ThreadId>,
    pub from: Option<EmailAddress>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    pub received_at: DateTime<Utc>,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    /// `None` for ordinary mail. A `bool` was not enough (#1491): Drafts and the Outbox hold five
    /// distinct states.
    pub send_state: Option<DraftState>,
    /// `None` for everything else, including a send waiting only on the drainer.
    pub send_at: Option<DateTime<Utc>>,
    pub has_attachments: bool,
    pub size: u64,
    /// A column of the list query: a lookup per row through `ThreadRepository::get` cost five
    /// statements per row (#1613).
    pub thread_count: Option<u32>,
    /// `received_at` when filed, the wake time once a snooze has woken it.
    pub sort_at: DateTime<Utc>,
}

impl MessageListRow {
    pub fn is_draft(&self) -> bool {
        self.send_state.is_some()
    }

    pub fn cursor(&self) -> ListCursor {
        ListCursor {
            received_at: self.received_at,
            sort_at: self.sort_at,
            id: self.id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    pub received_at: DateTime<Utc>,
    pub sort_at: DateTime<Utc>,
    /// Breaks ties between messages received in the same millisecond, making the order total.
    pub id: MessageId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListQuery {
    pub scope: ListScope,
    pub limit: u32,
    pub after: Option<ListCursor>,
}

impl ListQuery {
    pub fn mailbox(id: MailboxId) -> Self {
        Self::new(ListScope::Mailbox(id))
    }

    pub fn account(id: AccountId) -> Self {
        Self::new(ListScope::Account(id))
    }

    pub fn flagged(id: AccountId) -> Self {
        Self::new(ListScope::Flagged(id))
    }

    pub fn snoozed(id: AccountId) -> Self {
        Self::new(ListScope::Snoozed(id))
    }

    /// Not a mailbox: the rows are filed in Drafts and `send_state` picks them out (ADR 0036).
    pub fn outbox(id: AccountId) -> Self {
        Self::new(ListScope::Outbox(id))
    }

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

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    pub fn after(mut self, cursor: ListCursor) -> Self {
        self.after = Some(cursor);
        self
    }
}

/// Not a `Vec<MessageId>`: `Ctrl+A` over a huge mailbox must reach the statement still shaped like
/// a query, so archiving a whole mailbox is one `UPDATE` over an index. Both variants render to a
/// `WHERE` fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSet {
    /// Rows hidden pending a remote delete are excluded: the set means what the list shows.
    InMailbox {
        mailbox: MailboxId,
        except: Vec<MessageId>,
    },
    /// Focus's membership, not the folder's: mail held for a digest is not listed
    /// ([`super::focus_excludes`]). What is taken back out is conversations: every one a deselected
    /// row stands for, in whichever account this unit is.
    InFocusInbox {
        mailbox: MailboxId,
        except: Vec<MessageId>,
        except_threads: Vec<ThreadId>,
    },
    /// Not stable across the action it accompanies when that action writes the flag it selects on;
    /// see [`MessageSet::predicate`] for the ordering (#52).
    Flagged {
        account: AccountId,
        except: Vec<MessageId>,
    },
    /// How undo takes back a bulk action without naming its rows: the queue wrote one row per
    /// message in one numbered run, so the run is the set. See [`OperationRange`].
    Queued(OperationRange),
    /// Makes a bulk flag write exact: only rows that disagree are changed, and the queue carries
    /// precisely those. A queue row for an already-read message would put it inside the run undo
    /// takes back, so `u` would clear a flag the action never set. Narrowing is a column
    /// comparison, so the set stays a predicate.
    WithFlag {
        set: Box<MessageSet>,
        flag: ColumnFlag,
        present: bool,
    },
    InSourceMailbox {
        set: Box<MessageSet>,
        mailbox: MailboxId,
    },
}

impl MessageSet {
    pub fn in_mailbox(mailbox: MailboxId) -> Self {
        MessageSet::InMailbox {
            mailbox,
            except: Vec::new(),
        }
    }

    /// `None` for [`MessageSet::Queued`], whose rows are wherever the operation put them.
    pub fn mailbox(&self) -> Option<MailboxId> {
        match self {
            MessageSet::InMailbox { mailbox, .. } | MessageSet::InFocusInbox { mailbox, .. } => {
                Some(*mailbox)
            }
            MessageSet::Flagged { .. } | MessageSet::Queued(_) => None,
            MessageSet::InSourceMailbox { mailbox, .. } => Some(*mailbox),
            MessageSet::WithFlag { set, .. } => set.mailbox(),
        }
    }

    pub fn flagged(account: AccountId) -> Self {
        MessageSet::Flagged {
            account,
            except: Vec::new(),
        }
    }

    /// What a move out of a smart folder groups by: `Operation::Move` carries one `from` per run,
    /// so a set spanning folders is enqueued a folder at a time.
    pub fn within(self, mailbox: MailboxId) -> Self {
        MessageSet::InSourceMailbox {
            set: Box::new(self),
            mailbox,
        }
    }

    pub fn with_flag(self, flag: ColumnFlag, present: bool) -> Self {
        MessageSet::WithFlag {
            set: Box::new(self),
            flag,
            present,
        }
    }

    /// Numbered parameters from `?first` upwards; the fragment always constrains `messages`.
    pub(crate) fn predicate(&self, first: usize) -> (String, Vec<i64>) {
        match self {
            MessageSet::InMailbox { mailbox, except } => {
                let mut sql =
                    format!("messages.mailbox_id = ?{first} AND messages.deleted_locally = 0");
                sql.push_str(&without_conversations(except, first + 1));
                let mut arguments = vec![mailbox.get()];
                arguments.extend(except.iter().map(|id| id.get()).collect::<Vec<_>>());
                (sql, arguments)
            }
            MessageSet::InFocusInbox {
                mailbox,
                except,
                except_threads,
            } => {
                let mut sql = format!(
                    "messages.mailbox_id = ?{first} AND messages.{}{}",
                    super::VISIBLE,
                    super::focus_excludes("messages.")
                );
                let mut next = first + 1;
                if !except.is_empty() {
                    sql.push_str(&format!(
                        " AND messages.id NOT IN ({})",
                        placeholders(except.len(), next)
                    ));
                    next += except.len();
                }
                if !except_threads.is_empty() {
                    sql.push_str(&format!(
                        " AND (messages.thread_id IS NULL OR messages.thread_id NOT IN ({}))",
                        placeholders(except_threads.len(), next)
                    ));
                }
                let mut arguments = vec![mailbox.get()];
                arguments.extend(except.iter().map(|id| id.get()));
                arguments.extend(except_threads.iter().map(|id| id.get()));
                (sql, arguments)
            }
            // Bounded by two integers, so SQLite seeks the queue's primary key.
            MessageSet::Queued(range) => (
                format!(
                    "messages.id IN (SELECT target_id FROM operation_queue
                                      WHERE id BETWEEN ?{first} AND ?{}
                                        AND target_kind = 'message')",
                    first + 1
                ),
                vec![range.first.get(), range.last.get()],
            ),
            // Deliberately not stable across the write it accompanies: an unflag over Flagged would
            // empty its own predicate halfway through the statement. So `enqueue_set` must run
            // before the write in every bulk verb.
            MessageSet::Flagged { account, except } => {
                let mut sql = format!(
                    "messages.account_id = ?{first} AND messages.flagged = 1 \
                     AND messages.deleted_locally = 0"
                );
                if !except.is_empty() {
                    sql.push_str(&format!(
                        " AND messages.id NOT IN ({})",
                        placeholders(except.len(), first + 1)
                    ));
                }
                let mut arguments = vec![account.get()];
                arguments.extend(except.iter().map(|id| id.get()).collect::<Vec<_>>());
                (sql, arguments)
            }
            MessageSet::InSourceMailbox { set, mailbox } => {
                let (inner, mut arguments) = set.predicate(first);
                let next = first + arguments.len();
                arguments.push(mailbox.get());
                (
                    format!("({inner}) AND messages.mailbox_id = ?{next}"),
                    arguments,
                )
            }
            MessageSet::WithFlag { set, flag, present } => {
                let (inner, mut arguments) = set.predicate(first);
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

/// `\Seen` and `\Flagged` are stored twice, in `flags` and as a boolean column, so list and sidebar
/// filter and count without parsing text. The column is both the predicate selecting rows that
/// disagree and half the write. Other flags have no bulk write: there is no column to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFlag {
    Seen,
    Flagged,
}

impl ColumnFlag {
    pub fn of(flag: &Flag) -> Option<Self> {
        match flag {
            Flag::Seen => Some(ColumnFlag::Seen),
            Flag::Flagged => Some(ColumnFlag::Flagged),
            _ => None,
        }
    }

    pub fn flag(self) -> Flag {
        match self {
            ColumnFlag::Seen => Flag::Seen,
            ColumnFlag::Flagged => Flag::Flagged,
        }
    }

    fn column(self) -> &'static str {
        match self {
            ColumnFlag::Seen => "seen",
            ColumnFlag::Flagged => "flagged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagSource {
    Local,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredServerState {
    pub id: MessageId,
    pub uid: Uid,
    pub remote_id: Option<RemoteId>,
    pub mod_seq: Option<ModSeq>,
    pub flags: FlagSet,
    pub thread_id: Option<ThreadId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertReport {
    pub inserted: usize,
    /// The messages this batch wrote a row for, not ones it matched and updated (including a sent
    /// copy adopted by Message-ID): counts a message once, on the pass that first filed it.
    pub inserted_ids: Vec<MessageId>,
    pub updated: usize,
    /// Reported so "the resync undid my read" is diagnosable without a debugger (#317).
    pub flags_preserved: usize,
    /// Nothing on the wire carries a label yet (#780).
    pub labels_preserved: usize,
    pub own_drafts: usize,
    /// Re-creating the row would put back something the user watched leave. See
    /// [`MessageRepository::upsert_batch`] and #368.
    pub shadowed_by_pending: usize,
}

/// `None` is no such part; `Some("")` is a part that exists and is empty: the reading pane
/// distinguishes them, and collapsing them made a body-less message look like one still downloading
/// (#70).
///
/// Not on [`Message`]: a body on every list row would be the whole mailbox in memory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]

pub struct StoredBody {
    pub text: Option<String>,
    pub html: Option<String>,
    pub headers: Option<String>,
    /// Part of the body, not a separate column: the block and its partialness are one fact, and a
    /// writer that could set them apart eventually would.
    pub headers_truncated: bool,
    /// [`postio_model::mime::ParsedMessage::encoding_problems`], carried so a reader can say so;
    /// part of `StoredBody` so nobody builds one without answering it (#901).
    pub encoding_problems: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRepair {
    pub message_id: MessageId,
    /// `Some` means the block is recoverable from disk with no network; `None` means a fetch is the
    /// only way (the `partial` state, or an evicted raw source).
    pub raw_blob_id: Option<BlobId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillCandidate {
    pub message_id: MessageId,
    pub mailbox_id: MailboxId,
    pub mailbox_path: String,
    pub uid: Uid,
    pub remote_id: RemoteId,
    pub size: u64,
    /// [`size`](Self::size) less the attachment parts `BODYSTRUCTURE` reported, which a text-axis
    /// fetch leaves on the server (ADR 0017). The background lane's size cap is measured against
    /// it, so a short note carrying a big PDF still has its words fetched. An estimate; equals
    /// `size` when the structure was never recorded.
    pub text_size: u64,
    pub received_at: DateTime<Utc>,
    pub mailbox_role: MailboxRole,
}

impl BackfillCandidate {
    fn with_text_size(mut self, text_size: u64) -> Self {
        self.text_size = text_size.min(self.size);
        self
    }
}

/// Known before anything is downloaded: `BODYSTRUCTURE` arrives with the header sync (ADR 0017).
/// Bytes are wire sizes, still base64-inflated, not size on disk;
/// [`BlobStore::len_of`](crate::BlobStore::len_of) answers that.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageFootprint {
    pub messages: u64,
    pub total_bytes: u64,
    /// Around 90% on a real account, and the reason the two axes exist.
    pub attachment_bytes: u64,
    pub local_bytes: u64,
    /// `false` means these numbers are a lower bound and will grow; a surface must say so.
    pub complete: bool,
}

impl StorageFootprint {
    pub fn text_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.attachment_bytes)
    }

    pub fn is_empty(&self) -> bool {
        self.messages == 0
    }
}

#[derive(Debug)]
pub struct MessageRepository<'a> {
    connection: &'a Connection,
}

const MESSAGE_COLUMNS: &str = "\
id, account_id, mailbox_id, thread_id, rfc_message_id, in_reply_to, reference_ids, subject,
date, received_at, preview, size, flags, has_attachments, uid, uid_validity, mod_seq,
remote_id, body_state, flags_dirty, has_pending_operations, deleted_locally, last_synced_at,
raw_blob_id, content_type, list_id, text_part_id, text_part_headers,
html_part_id, html_part_headers, snoozed_until, text_is_flowed, read_receipt_requested,
unsubscribe_offered, automation, content_namespace, content_key";

/// Well under `SQLITE_MAX_VARIABLE_NUMBER`; one chunk is the normal case.
const ID_CHUNK: usize = 500;

/// The sender comes from a correlated lookup, not a join, so the plan is "walk the list index, then
/// one seek per row shown", bounded by the window. Its name is the live owner's user-given name if
/// any, else the header's: two more primary-key seeks per row.
pub(crate) const LIST_COLUMNS: &str = concat!(
    "\
messages.id, messages.thread_id, messages.subject, messages.preview, messages.received_at,
messages.seen, messages.flagged, messages.answered, messages.draft, messages.has_attachments,
messages.size, messages.send_state, messages.send_at,
(SELECT coalesce(",
    crate::shown_name!("named"),
    ", recipients.name) FROM recipients
    JOIN addresses ON addresses.id = recipients.address_id
    LEFT JOIN contacts named ON named.id = addresses.contact_id AND named.state = 'live'
  WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
  ORDER BY recipients.position LIMIT 1),
(SELECT addresses.address FROM recipients
    JOIN addresses ON addresses.id = recipients.address_id
  WHERE recipients.message_id = messages.id AND recipients.kind = 'from'
  ORDER BY recipients.position LIMIT 1),
(SELECT threads.message_count FROM threads WHERE threads.id = messages.thread_id),
messages.sort_at"
);

/// Rewritten from the one list, never copied, for the reason `ThreadRepository`'s
/// `latest_messages_for` gives.
pub(crate) fn list_columns_of(alias: &str) -> String {
    LIST_COLUMNS.replace("messages.", &format!("{alias}."))
}

impl<'a> MessageRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Body and header bytes go through [`MessageRepository::set_body`]; an immutable backend
    /// identity can reuse an existing owner's bytes and availability.
    pub async fn create(&self, message: &mut Message) -> Result<MessageId> {
        sql::in_scope(self.connection, |transaction| async move {
            let id = insert(&transaction, message).await?;
            message.id = id;
            write_children(&transaction, message).await?;
            Ok(id)
        })
        .await
    }

    /// Takes `&mut` because attachment rows are rewritten and re-issued, which would leave the
    /// value holding stale ids.
    pub async fn update(&self, message: &mut Message) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            write_update(&transaction, message).await?;
            Ok(())
        })
        .await
    }

    /// One transaction: a resync of a thousand messages is otherwise a thousand `fsync`s, and a
    /// half-applied page is a mailbox the next sync must reconcile with itself. A message is
    /// already known by `(mailbox, UIDVALIDITY, UID)`.
    ///
    /// Takes a `Vec` and shortens it, so the batch means "what was stored" for the callers that go
    /// on to thread each message and record correspondents. It drops:
    /// - the server's copy of a draft this client wrote, which the composer owns (#51); a draft
    /// from another client has no local draft row and stays ordinary mail;
    /// - a message with an undrained `Move` or `Delete` out of this mailbox (#368): archiving is
    /// local-first, so a resync would otherwise re-create the row and the archived message would
    /// reappear until the queue drains. The shadow is keyed on the queue row's snapshot of server
    /// coordinates (#289) and lifts when the operation settles, `Done` or `Failed`, so a refused
    /// move cannot lose mail.
    pub async fn upsert_batch(&self, batch: &mut Vec<Message>) -> Result<UpsertReport> {
        sql::in_scope(self.connection, |transaction| async move {
            let mut report = UpsertReport::default();

            // Read once for the batch: a store holds a handful of drafts, a page hundreds of
            // messages.
            let mine = own_draft_copies(&transaction).await?;
            let before = batch.len();
            batch.retain(|message| match &message.server.remote_id {
                Some(remote_id) => {
                    !mine.contains(&(message.mailbox_id, remote_id.as_str().to_owned()))
                }
                None => true,
            });
            report.own_drafts = before - batch.len();

            let shadowed = shadowed_by_pending_operation(&transaction).await?;
            let before = batch.len();
            batch.retain(|message| match &message.server.remote_id {
                Some(remote_id) => {
                    !shadowed.contains(&(message.mailbox_id, remote_id.as_str().to_owned()))
                }
                None => true,
            });
            report.shadowed_by_pending = before - batch.len();

            let unacknowledged = unacknowledged_flag_changes(&transaction).await?;

            for message in batch.iter_mut() {
                // The identity names a message (#543); the wire pair is the fallback for rows
                // synced before it. Matching the pair first would let a backend with synthetic uids
                // (#544) insert a second copy.
                let by_identity = match &message.server.remote_id {
                    Some(remote_id) => {
                        find_by_remote_id(&transaction, message.mailbox_id, remote_id).await?
                    }
                    None => None,
                };
                let existing = match (by_identity, message.server.uid, message.server.uid_validity)
                {
                    (Some(id), _, _) => Some(id),
                    (None, Some(uid), Some(validity)) => {
                        find_by_generation_uid(
                            &transaction,
                            message.mailbox_id,
                            Generation::new(validity.get()),
                            uid,
                        )
                        .await?
                    }
                    _ => None,
                };
                // Last, for a row with no server coordinates: the copy this client wrote before the
                // server named it (#942). After both server-identity routes, so a message the
                // server can place is never resolved by `Message-ID`.
                let existing = match (existing, &message.rfc_message_id) {
                    (None, Some(rfc)) => {
                        find_awaiting_identity(
                            &transaction,
                            message.account_id,
                            message.mailbox_id,
                            rfc,
                        )
                        .await?
                    }
                    // No `Message-ID`: a server that answered the move without a `COPYUID` (#1994).
                    (None, None) => {
                        find_awaiting_headerless(
                            &transaction,
                            message.account_id,
                            message.mailbox_id,
                            message.received_at,
                            message.size,
                        )
                        .await?
                    }
                    (existing, _) => existing,
                };

                match existing {
                    Some(id) => {
                        message.id = id;
                        // The server's flags plus what the queue still holds for this message
                        // (#317): otherwise a `CHANGEDSINCE` pass before the drainer writes the
                        // server's stale copy over a local read.
                        if let Some(changes) = unacknowledged.get(&id) {
                            message.flags = with_unacknowledged(&message.flags, changes);
                            report.flags_preserved += 1;
                        }
                        // Labels are local-only: nothing on the wire carries them, so an empty set
                        // means this sync has nothing to say, not that the server says none (#780).
                        // A set a sync supplies wins.
                        if message.labels.is_empty() {
                            message.labels = read_labels(&transaction, id).await?;
                            if !message.labels.is_empty() {
                                report.labels_preserved += 1;
                            }
                        }
                        write_update(&transaction, message).await?;
                        report.updated += 1;
                    }
                    None => {
                        message.id = insert(&transaction, message).await?;
                        write_children(&transaction, message).await?;
                        report.inserted += 1;
                        report.inserted_ids.push(message.id);
                    }
                }
                // Another machine's filing, carried on the message (#2076); no statement unless the
                // flags carry a reason's keyword.
                super::FilterDecisionRepository::new(&transaction)
                    .follow(message.id, &message.flags)
                    .await?;
            }

            Ok(report)
        })
        .await
    }

    /// `None` for ordinary mail. Separate from [`Message`]: only the reading pane asks (#1525), and
    /// widening the message read would cost every caller.
    pub async fn send_state(&self, id: MessageId) -> Result<Option<DraftState>> {
        let state: Option<String> = sql::first(
            self.connection,
            "SELECT send_state FROM messages WHERE id = ?1",
            [id.get()],
            |row| row.opt_text(0),
        )
        .await?
        .flatten();
        state
            .map(|state| {
                DraftState::from_name(&state)
                    .ok_or_else(|| unknown_enum("messages.send_state", &state))
            })
            .transpose()
    }

    /// The body and headers are empty; see [`MessageRepository::body`].
    pub async fn get(&self, id: MessageId) -> Result<Option<Message>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        )
        .await?;
        let found = crate::sql::first_of(&mut statement, [id.get()], read_message).await?;
        let Some(mut message) = found else {
            return Ok(None);
        };
        drop(statement);

        read_recipients(self.connection, &mut message).await?;
        message.attachments = read_attachments(self.connection, id).await?;
        message.labels = read_labels(self.connection, id).await?;
        Ok(Some(message))
    }

    /// Ids are never reused (`AUTOINCREMENT`), so every message stored after it has a larger one:
    /// what Focus marks the mail it has accounted for with. One lookup at the end of the key.
    pub async fn newest_id(&self) -> Result<Option<MessageId>> {
        sql::first(
            self.connection,
            "SELECT id FROM messages ORDER BY id DESC LIMIT 1",
            (),
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// What a sweep of the inbox walks a window at a time, so the inbox is never read whole. One
    /// statement, a seek on `idx_messages_list`.
    pub async fn focus_inbox_window(
        &self,
        mailbox: MailboxId,
        after: Option<(DateTime<Utc>, MessageId)>,
        limit: u32,
    ) -> Result<Vec<(MessageId, DateTime<Utc>)>> {
        let (at, id) = after.map_or((i64::MAX, i64::MAX), |(at, id)| (to_millis(at), id.get()));
        sql::all(
            self.connection,
            &Self::explain_focus_inbox_window(),
            bind![mailbox.get(), at, id, i64::from(limit)],
            |row| Ok((MessageId::new(row.col(0)?), from_millis(row.col(1)?))),
        )
        .await
    }

    pub fn explain_focus_inbox_window() -> String {
        format!(
            "SELECT id, sort_at FROM messages
              WHERE mailbox_id = ?1 AND {}{} AND (sort_at, id) < (?2, ?3)
              ORDER BY sort_at DESC, id DESC LIMIT ?4",
            super::VISIBLE,
            super::focus_excludes("messages.")
        )
    }

    pub async fn page(&self, query: &ListQuery) -> Result<Vec<MessageListRow>> {
        let sql = self.explain(query);
        sql::all(self.connection, &sql, page_arguments(query), read_list_row).await
    }

    /// Prefer [`MessageRepository::page`]: an offset counts from the top every time, so a message
    /// arriving while the user scrolls shifts every row and this window silently skips one.
    pub async fn page_at(&self, query: &ListQuery, offset: u32) -> Result<Vec<MessageListRow>> {
        let sql = format!("{} OFFSET {offset}", self.explain(query));
        sql::all(self.connection, &sql, page_arguments(query), read_list_row).await
    }

    /// For search hits, which no [`ListScope`] fits: ranked, spanning folders, with no offset.
    ///
    /// The order is restored after the read: SQL answers `IN` in primary-key order, close enough to
    /// date order that a lost ranking would look right in fixtures.
    ///
    /// Ids the store no longer holds are dropped: the index and store may disagree for a moment.
    pub async fn rows_for(&self, ids: &[MessageId]) -> Result<Vec<MessageListRow>> {
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
            let mut statement = sql::statement(self.connection, &sql).await?;
            let rows = sql::mapped(
                &mut statement,
                chunk.iter().map(|id| id.get()).collect::<Vec<_>>(),
                read_list_row,
            )
            .await?;
            for row in rows {
                found.insert(row.id, row);
            }
        }

        Ok(ids.iter().filter_map(|id| found.get(id).cloned()).collect())
    }

    pub async fn count(&self, query: &ListQuery) -> Result<u32> {
        let sql = format!(
            "SELECT count(*) FROM messages WHERE {}",
            where_clause(query, false)
        );
        let arguments = scope_arguments(&query.scope);
        let count: i64 = sql::one(self.connection, &sql, arguments, |row| row.col(0)).await?;
        Ok(count as u32)
    }

    /// Exposed so a test can put `EXPLAIN QUERY PLAN` in front of the hot-path list query.
    pub fn explain(&self, query: &ListQuery) -> String {
        format!(
            "SELECT {LIST_COLUMNS} FROM messages WHERE {} \
             ORDER BY messages.{key} DESC, messages.id DESC LIMIT {}",
            where_clause(query, query.after.is_some()),
            query.limit,
            key = order_key(&query.scope),
        )
    }

    pub async fn set_flags(
        &self,
        id: MessageId,
        flags: &FlagSet,
        source: FlagSource,
    ) -> Result<()> {
        let flags = flags.persistable();
        let changed = sql::execute(
            self.connection,
            "UPDATE messages
                SET flags = ?2, seen = ?3, flagged = ?4, answered = ?5, draft = ?6,
                    deleted = ?7, flags_dirty = ?8
              WHERE id = ?1",
            bind![
                id.get(),
                flag_text(&flags),
                flags.is_seen(),
                flags.is_flagged(),
                flags.is_answered(),
                flags.is_draft(),
                flags.is_deleted(),
                source == FlagSource::Local,
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// The server identity is cleared: a UID belongs to the mailbox that issued it, and keeping it
    /// would make the next resync match the wrong message. The rows are marked as having pending
    /// operations.
    pub async fn move_to(&self, ids: &[MessageId], mailbox_id: MailboxId) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages
                SET mailbox_id = ?1, uid = NULL, uid_validity = NULL, mod_seq = NULL,
                    remote_id = NULL, has_pending_operations = 1,
                    rfc_message_id = lower(rfc_message_id)
              WHERE id IN ({})",
            placeholders(ids.len(), 2)
        );
        let mut arguments = vec![mailbox_id.get()];
        arguments.extend(ids.iter().map(|id| id.get()).collect::<Vec<_>>());
        Ok(sql::execute(self.connection, &sql, arguments).await? as usize)
    }

    /// [`move_to`] clears a row's coordinates, and a later sync can only re-attach the server's
    /// copy by `Message-ID`, which a message without one lacks, so its copy became a second row
    /// (#1983). The server's `COPYUID` names the copy exactly.
    ///
    /// Only a row still in `mailbox_id` with no identity at all is written. Returns whether a row
    /// was written.
    ///
    /// [`move_to`]: MessageRepository::move_to
    pub async fn adopt_moved_identity(
        &self,
        id: MessageId,
        mailbox_id: MailboxId,
        uid: Uid,
        uid_validity: UidValidity,
        remote_id: &RemoteId,
    ) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE messages
                SET uid = ?1, uid_validity = ?2, remote_id = ?3
              WHERE id = ?4 AND mailbox_id = ?5
                AND uid IS NULL AND remote_id IS NULL",
            bind![
                i64::from(uid.get()),
                i64::from(uid_validity.get()),
                remote_id.as_str(),
                id.get(),
                mailbox_id.get()
            ],
        )
        .await?;
        Ok(changed > 0)
    }

    /// One indexed `SELECT DISTINCT` over the write's predicate, so it costs the folders the set
    /// spans, not the messages. A move out of a smart folder needs it because `Operation::Move`
    /// carries one `from` per run; see [`MessageSet::within`].
    pub async fn mailboxes_of_set(&self, set: &MessageSet) -> Result<Vec<MailboxId>> {
        let (predicate, arguments) = set.predicate(1);
        let sql = format!("SELECT DISTINCT messages.mailbox_id FROM messages WHERE {predicate}");
        let mut statement = sql::statement(self.connection, &sql).await?;
        let rows = sql::mapped(&mut statement, arguments, |row| {
            row.col::<i64>(0).map(MailboxId::new)
        })
        .await?;
        Ok(rows)
    }

    /// One `UPDATE` over the mailbox index with nothing read first, whatever the mailbox holds. The
    /// server identity is cleared as [`move_to`] does.
    ///
    /// [`move_to`]: MessageRepository::move_to
    pub async fn move_set(&self, set: &MessageSet, mailbox_id: MailboxId) -> Result<usize> {
        let (predicate, arguments) = set.predicate(2);
        let sql = format!(
            "UPDATE messages
                SET mailbox_id = ?1, uid = NULL, uid_validity = NULL, mod_seq = NULL,
                    remote_id = NULL, has_pending_operations = 1
              WHERE {predicate}"
        );
        let mut parameters = vec![mailbox_id.get()];
        parameters.extend(arguments);
        Ok(sql::execute(self.connection, &sql, parameters).await? as usize)
    }

    /// One indexed `count(*)`, for the undo toast's number.
    pub async fn count_set(&self, set: &MessageSet) -> Result<u32> {
        let (predicate, arguments) = set.predicate(1);
        let sql = format!("SELECT count(*) FROM messages WHERE {predicate}");
        let count: i64 = sql::one(self.connection, &sql, arguments, |row| row.col(0)).await?;
        Ok(count as u32)
    }

    /// One `UPDATE` with nothing read.
    ///
    /// The flags text is rebuilt without reading it: the five system flags with columns (`\Seen`,
    /// `\Answered`, `\Flagged`, `\Deleted`, `\Draft`) are the five lowest-ranked persistable flags,
    /// so the text is those five in column order followed by keywords. The statement rebuilds the
    /// head from the booleans and keeps the tail by stripping the five spellings; appending would
    /// put `\Seen` last and break the schema's ordering.
    ///
    /// Rows that already agree are still written unless the caller narrowed with
    /// [`MessageSet::with_flag`]; the queue rows and undo entry must cover the same rows.
    ///
    /// [`set_flags`]: MessageRepository::set_flags
    pub async fn set_flag_on_set(
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
        Ok(sql::execute(self.connection, &sql, parameters).await? as usize)
    }

    /// For when a first sync did not ask for them. A message expunged meanwhile is no error.
    pub async fn set_promoted(&self, id: MessageId, promoted: PromotedHeaders) -> Result<bool> {
        let written = sql::execute(
            self.connection,
            "UPDATE messages SET unsubscribe_offered = ?2, automation = ?3 WHERE id = ?1",
            bind![
                id.get(),
                i64::from(promoted.unsubscribe_offered),
                i64::from(promoted.automation)
            ],
        )
        .await?;
        Ok(written > 0)
    }

    /// The row stays and the list stops showing it: what makes delete instant and undo possible.
    pub async fn set_deleted_locally(&self, ids: &[MessageId], deleted: bool) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages SET deleted_locally = ?1, has_pending_operations = ?1
              WHERE id IN ({})",
            placeholders(ids.len(), 2)
        );
        let mut arguments = vec![i64::from(deleted)];
        arguments.extend(ids.iter().map(|id| id.get()).collect::<Vec<_>>());
        Ok(sql::execute(self.connection, &sql, arguments).await? as usize)
    }

    /// Never touched by `insert`/`write_update`: a row is never born snoozed, so a header sync has
    /// nothing to clobber (migration 0021).
    pub async fn snooze(&self, ids: &[MessageId], until: DateTime<Utc>) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages SET snoozed_until = ?1 WHERE id IN ({})",
            placeholders(ids.len(), 2)
        );
        let mut arguments = vec![to_millis(until)];
        arguments.extend(ids.iter().map(|id| id.get()).collect::<Vec<_>>());
        Ok(sql::execute(self.connection, &sql, arguments).await? as usize)
    }

    /// What a new reply does to a snoozed conversation (spec 013 D3). A thread with nothing asleep
    /// costs the statement and no row (`idx_messages_thread` covers `snoozed_until`).
    pub async fn unsnooze_threads(&self, threads: &[ThreadId]) -> Result<usize> {
        if threads.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages SET snoozed_until = NULL
              WHERE thread_id IN ({}) AND snoozed_until IS NOT NULL",
            placeholders(threads.len(), 1)
        );
        Ok(sql::execute(
            self.connection,
            &sql,
            threads.iter().map(|id| id.get()).collect::<Vec<_>>(),
        )
        .await? as usize)
    }

    pub async fn unsnooze(&self, ids: &[MessageId]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "UPDATE messages SET snoozed_until = NULL WHERE id IN ({})",
            placeholders(ids.len(), 1)
        );
        Ok(sql::execute(
            self.connection,
            &sql,
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
        )
        .await? as usize)
    }

    /// Scoped to one account: each account's engine ticks on its own `POLL_INTERVAL`, and an
    /// unscoped sweep would have them racing over each other's rows.
    ///
    /// Visibility never depends on this running: queries read `snoozed_until` against wall-clock
    /// time. This clears the column (so the `mailboxes.snoozed_count` trigger fires) and names what
    /// to repaint.
    pub async fn wake_due(&self, account: AccountId, now: DateTime<Utc>) -> Result<Vec<MailboxId>> {
        let now_millis = to_millis(now);
        // Cached: this runs every five seconds for the life of the process, and preparing it afresh
        // put `sqlite3RunParser` into an idle profile (#1237).
        // No `ORDER BY` in the SQL: with one the planner declines `idx_messages_snoozed_due` and
        // scans `idx_messages_list`, the walk #1237 removed. The sort is done in Rust over at most
        // the account's mailbox count.
        let mut statement = sql::statement(
            self.connection,
            "SELECT DISTINCT mailbox_id FROM messages
              WHERE account_id = ?1 AND snoozed_until IS NOT NULL AND snoozed_until <= ?2",
        )
        .await?;
        let mut woken: Vec<MailboxId> =
            sql::mapped(&mut statement, bind![account.get(), now_millis], |row| {
                Ok(MailboxId::new(row.col(0)?))
            })
            .await?;
        woken.sort_unstable();

        if !woken.is_empty() {
            // A woken message comes back at the top (spec 007, research R7): its folder sort and
            // its conversation's account-wide sort become the wake time. First, while the snoozes
            // still say which.
            sql::execute(
                self.connection,
                "UPDATE threads
                    SET last_at = max(last_at, (
                        SELECT max(snoozed_until) FROM messages
                         WHERE messages.thread_id = threads.id AND account_id = ?1
                           AND snoozed_until IS NOT NULL AND snoozed_until <= ?2))
                  WHERE id IN (SELECT thread_id FROM messages
                                WHERE account_id = ?1 AND snoozed_until IS NOT NULL
                                  AND snoozed_until <= ?2 AND thread_id IS NOT NULL)",
                bind![account.get(), now_millis],
            )
            .await?;
            sql::execute(
                self.connection,
                "UPDATE messages SET sort_at = max(sort_at, snoozed_until), snoozed_until = NULL
                  WHERE account_id = ?1 AND snoozed_until IS NOT NULL AND snoozed_until <= ?2",
                bind![account.get(), now_millis],
            )
            .await?;
        }
        Ok(woken)
    }

    /// For an expunge the server has confirmed. Recipients, attachments and label links cascade;
    /// blobs are swept separately.
    pub async fn delete(&self, ids: &[MessageId]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut deleted = 0;
        for chunk in ids.chunks(ID_LOOKUP_CHUNK) {
            let sql = format!(
                "DELETE FROM messages WHERE id IN ({})",
                placeholders(chunk.len(), 1)
            );
            deleted += sql::execute(
                self.connection,
                &sql,
                chunk.iter().map(|id| id.get()).collect::<Vec<_>>(),
            )
            .await? as usize;
        }
        Ok(deleted)
    }

    pub async fn set_thread(&self, id: MessageId, thread_id: Option<ThreadId>) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            bind![id.get(), thread_id.map(ThreadId::get)],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    pub async fn by_uid(
        &self,
        mailbox_id: MailboxId,
        generation: Generation,
        uid: Uid,
    ) -> Result<Option<Message>> {
        match find_by_generation_uid(self.connection, mailbox_id, generation, uid).await? {
            Some(id) => self.get(id).await,
            None => Ok(None),
        }
    }

    /// One statement per [`ID_LOOKUP_CHUNK`] uids, ids only: [`by_uid`](Self::by_uid) pays two
    /// queries per uid.
    pub async fn ids_by_uid(
        &self,
        mailbox_id: MailboxId,
        generation: Generation,
        uids: &[Uid],
    ) -> Result<Vec<MessageId>> {
        let mut ids = Vec::with_capacity(uids.len());
        for chunk in uids.chunks(ID_LOOKUP_CHUNK) {
            let sql = format!(
                "SELECT id FROM messages
                  WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid IN ({})",
                placeholders(chunk.len(), 3)
            );
            let mut params: Vec<i64> = vec![mailbox_id.get(), i64::from(generation.get())];
            params.extend(chunk.iter().map(|uid| i64::from(uid.get())));
            ids.extend(
                sql::all_unbounded(self.connection, &sql, params, |row| {
                    Ok(MessageId::new(row.col(0)?))
                })
                .await?,
            );
        }
        Ok(ids)
    }

    /// `all_unbounded` on purpose: a resync diffs the whole local set against the server's, and a
    /// limit would silently stop diffing partway.
    pub async fn uids_in(&self, mailbox_id: MailboxId, generation: Generation) -> Result<Vec<Uid>> {
        sql::all_unbounded(
            self.connection,
            "SELECT uid FROM messages
              WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid IS NOT NULL
              ORDER BY uid",
            bind![mailbox_id.get(), i64::from(generation.get())],
            |row| Ok(Uid::new(row.col::<i64>(0)? as u32)),
        )
        .await
    }

    /// What a full resync compares a fetch against before writing (#1719): a backend with no MODSEQ
    /// re-reads every header each pass, and a row whose flags and identity match has nothing to
    /// write. One statement per batch.
    pub async fn server_states(
        &self,
        mailbox_id: MailboxId,
        generation: Generation,
        uids: &[Uid],
    ) -> Result<Vec<StoredServerState>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let list = format!(
            "[{}]",
            uids.iter()
                .map(|uid| uid.get().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut statement = sql::statement(
            self.connection,
            "SELECT m.id, m.uid, m.remote_id, m.mod_seq, m.flags, m.thread_id
               FROM json_each(?3) j
               CROSS JOIN messages m
                 ON m.mailbox_id = ?1 AND m.uid_validity = ?2 AND m.uid = j.value",
        )
        .await?;
        sql::mapped(
            &mut statement,
            bind![mailbox_id.get(), i64::from(generation.get()), list],
            |row| {
                Ok(StoredServerState {
                    id: MessageId::new(row.col(0)?),
                    uid: Uid::new(row.col::<i64>(1)? as u32),
                    remote_id: row.col::<Option<String>>(2)?.map(RemoteId::new),
                    mod_seq: row
                        .col::<Option<i64>>(3)?
                        .map(|seq| ModSeq::new(seq as u64)),
                    flags: parse_flags(&row.col::<String>(4)?),
                    thread_id: row.col::<Option<i64>>(5)?.map(ThreadId::new),
                })
            },
        )
        .await
    }

    /// `all_unbounded` for the reason [`uids_in`](Self::uids_in) gives.
    pub async fn flags_in(
        &self,
        mailbox_id: MailboxId,
        generation: Generation,
    ) -> Result<Vec<(MessageId, Uid, FlagSet)>> {
        sql::all_unbounded(
            self.connection,
            "SELECT id, uid, flags FROM messages
              WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid IS NOT NULL
              ORDER BY uid",
            bind![mailbox_id.get(), i64::from(generation.get())],
            |row| {
                Ok((
                    MessageId::new(row.col(0)?),
                    Uid::new(row.col::<i64>(1)? as u32),
                    parse_flags(&row.col::<String>(2)?),
                ))
            },
        )
        .await
    }

    /// The same merge [`upsert_batch`](Self::upsert_batch) applies to a `CHANGEDSINCE` pull (#317):
    /// the server's flags plus what the queue still holds, so a sweep before the drainer cannot
    /// write a stale copy over a local read. A message whose merged flags already match is not
    /// written or returned, so undrained local flags are not reported as a correction every pass.
    pub async fn apply_server_flags(
        &self,
        updates: &[(MessageId, FlagSet)],
    ) -> Result<Vec<MessageId>> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        sql::in_scope(self.connection, |transaction| async move {
            let unacknowledged = unacknowledged_flag_changes(&transaction).await?;
            let messages = MessageRepository::new(&transaction);
            let mut changed = Vec::new();
            for (id, server) in updates {
                let merged = match unacknowledged.get(id) {
                    Some(changes) => with_unacknowledged(server, changes),
                    None => server.clone(),
                }
                .persistable();
                let stored: Option<String> = sql::first(
                    &transaction,
                    "SELECT flags FROM messages WHERE id = ?1",
                    [id.get()],
                    |row| row.col(0),
                )
                .await?;
                let Some(stored) = stored else { continue };
                if parse_flags(&stored).persistable() == merged {
                    continue;
                }
                messages.set_flags(*id, &merged, FlagSource::Server).await?;
                super::FilterDecisionRepository::new(&transaction)
                    .follow(*id, &merged)
                    .await?;
                changed.push(*id);
            }
            Ok(changed)
        })
        .await
    }

    /// The server's copies of this client's drafts, which `upsert_batch` refuses, leave the mailbox
    /// permanently one row short of `EXISTS`. `sync::resync` re-enumerates a mailbox holding less
    /// than `EXISTS`, so without this it re-enumerated Drafts on every pass: a repair that cannot
    /// converge is a loop.
    pub async fn refused_rows_in(&self, mailbox: MailboxId) -> Result<u32> {
        let count: i64 = sql::one(
            self.connection,
            "SELECT count(*)
               FROM drafts
               JOIN mailboxes ON mailboxes.account_id = drafts.account_id
                             AND mailboxes.role = 'drafts'
              WHERE drafts.remote_id IS NOT NULL AND mailboxes.id = ?1",
            bind![mailbox.get()],
            |row| row.col(0),
        )
        .await?;
        Ok(count.max(0) as u32)
    }

    /// [`uids_in`](Self::uids_in)'s count without its allocation. Counts rows marked
    /// `deleted_locally` too, as `uids_in` lists them: the server still holds the message.
    pub async fn count_in(&self, mailbox_id: MailboxId, generation: Generation) -> Result<u32> {
        let count: i64 = sql::one(
            self.connection,
            "SELECT count(*) FROM messages
              WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid IS NOT NULL",
            bind![mailbox_id.get(), i64::from(generation.get())],
            |row| row.col(0),
        )
        .await?;
        Ok(count.max(0) as u32)
    }

    /// A list: `Message-ID` is not unique in the wild.
    pub async fn ids_by_rfc_message_id(
        &self,
        account_id: AccountId,
        rfc_message_id: &RfcMessageId,
    ) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            "SELECT id FROM messages
              WHERE account_id = ?1 AND rfc_message_id = ?2 COLLATE NOCASE
              ORDER BY id",
            bind![account_id.get(), rfc_message_id.as_str()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// A message that exists but was never downloaded answers `Some(StoredBody::default())`: the
    /// row is there and holds no parts, which is not the same as no row.
    ///
    /// Reads exactly three columns: the reading pane's 16 ms path. Each holds whichever shape
    /// [`crate::body_codec`] chose.
    pub async fn body(&self, id: MessageId) -> Result<Option<StoredBody>> {
        sql::first(
            self.connection,
            "SELECT message_contents.body_text, message_contents.body_html, message_contents.body_headers,
                    message_contents.body_headers_truncated, message_contents.body_encoding_problems
               FROM messages JOIN message_contents ON message_contents.id = messages.content_id
              WHERE messages.id = ?1",
            [id.get()],
            |row| {
                crate::store::BODY_LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let text: Option<Vec<u8>> = row.col(0)?;
                let html: Option<Vec<u8>> = row.col(1)?;
                Ok(StoredBody {
                    text: text.map(crate::body_codec::unpack),
                    html: html.map(crate::body_codec::unpack),
                    headers: row.col(2)?,
                    headers_truncated: row.col(3)?,
                    encoding_problems: row.col(4)?,
                })
            },
        )
        .await
    }

    /// Not on [`get`](Self::get): the block is part of the body (written by
    /// [`set_body`](Self::set_body) in the same statement), and every list row and reading-pane
    /// open would pay a decompression and a parse for a field only the indexer and `header:` want.
    ///
    /// A block never stored answers `Some` with an empty one: not a fault.
    ///
    /// # Errors
    ///
    /// [`Error::UnreadableBody`] if the stored block will not decode: an absent block and an
    /// unreadable one are opposite facts.
    pub async fn headers(&self, id: MessageId) -> Result<Option<postio_model::Headers>> {
        let Some(body) = self.body(id).await? else {
            return Ok(None);
        };
        Ok(Some(match body.headers {
            None => postio_model::Headers::new(),
            // `headers::parse_block` appends the terminator the stored block lacks, so a caller
            // that forgot would lose the last field (ADR 0025).
            Some(block) => postio_model::headers::parse_block(&block),
        }))
    }

    /// Every message with a body but no stored block (#884). Scoped to fetched bodies: one still
    /// waiting gets its block with it.
    ///
    /// Repairing a message must remove it from this answer ([`set_headers`](Self::set_headers)
    /// writes a non-NULL block): a candidate query that does not shrink spins a core for as long as
    /// the app is open (#500), so the caller compares batches and stops.
    ///
    /// Only rows that still have their raw source: others need a fetch, and mixing them in would
    /// let one batch of unfetchable rows make no progress and stop the pass before older fixable
    /// ones. Those are
    /// [`messages_needing_a_header_fetch`](Self::messages_needing_a_header_fetch)'s.
    pub async fn messages_missing_headers(&self, limit: u32) -> Result<Vec<HeaderRepair>> {
        sql::all(
            self.connection,
            "SELECT id, raw_blob_id FROM messages
              WHERE body_has_headers = 0
                AND raw_blob_id IS NOT NULL
                AND body_state IN ('partial', 'full')
              ORDER BY received_at DESC
              LIMIT ?1",
            [limit],
            |row| {
                Ok(HeaderRepair {
                    message_id: MessageId::new(row.col(0)?),
                    raw_blob_id: row.col::<Option<String>>(1)?.map(BlobId::new),
                })
            },
        )
        .await
    }

    /// `body_headers` NULL and no raw source: the `partial` state (sections fetched, no raw blob)
    /// or an evicted raw source. Separate from
    /// [`messages_missing_headers`](Self::messages_missing_headers): a network fetch must not share
    /// a batch with a local read.
    pub async fn messages_needing_a_header_fetch(
        &self,
        mailbox_id: MailboxId,
        limit: u32,
    ) -> Result<Vec<BackfillCandidate>> {
        sql::all(
            self.connection,
            NEEDING_A_HEADER_FETCH,
            bind![mailbox_id.get(), limit],
            |row| read_backfill_candidate(row, mailbox_id, role_at(row, 6)?),
        )
        .await
    }

    /// Not `set_body` with the other fields refilled: `set_body` replaces every part, and a header
    /// repair has only the headers, so it would clear the text and html.
    pub async fn set_headers(
        &self,
        id: MessageId,
        block: Option<&postio_model::headers::Block>,
    ) -> Result<()> {
        let encoded = block.map(|block| block.text.clone());
        let changed = sql::execute(
            self.connection,
            "UPDATE message_contents
                SET body_headers = ?2, body_headers_truncated = ?3
              WHERE id = (SELECT content_id FROM messages WHERE id = ?1)",
            bind![
                id.get(),
                encoded,
                block.is_some_and(|block| block.truncated),
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Takes the new [`BodyState`] with it: content and "how much is local" are one fact, and
    /// writing them separately would let the backfill queue disagree with the reader.
    ///
    /// **Replaces every part.** A `None` clears the column, so a refetch with no HTML part does not
    /// leave the previous HTML behind.
    ///
    /// Does not write the body's full-text row: that put every body commit through the tantivy
    /// index on the sync lane, where the next commit could inherit a segment merge of seconds
    /// (`fts_merge_stall`). The row is the indexer's:
    /// `postio_index::index::messages_missing_body_text` is the queue and
    /// `postio_session::spawn_body_indexer` drains it off the sync lane, so a body is searchable a
    /// moment after it lands.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if no such message.
    pub async fn set_body(
        &self,
        id: MessageId,
        body: &StoredBody,
        body_state: BodyState,
    ) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE message_contents
                    SET body_text = ?2, body_html = ?3, body_headers = ?4,
                        body_state = ?5,
                        body_headers_truncated = ?6, body_encoding_problems = ?7,
                        body_line_count = ?8, body_parsed_with = ?9,
                        content_type = (SELECT content_type FROM messages WHERE id = ?1),
                        text_part_id = (SELECT text_part_id FROM messages WHERE id = ?1),
                        text_part_headers = (SELECT text_part_headers FROM messages WHERE id = ?1),
                        html_part_id = (SELECT html_part_id FROM messages WHERE id = ?1),
                        html_part_headers = (SELECT html_part_headers FROM messages WHERE id = ?1),
                        text_is_flowed = (SELECT text_is_flowed FROM messages WHERE id = ?1),
                        read_receipt_requested = (SELECT read_receipt_requested FROM messages WHERE id = ?1)
                  WHERE id = (SELECT content_id FROM messages WHERE id = ?1)",
            bind![
                id.get(),
                body.text.as_deref().map(crate::body_codec::pack),
                body.html.as_deref().map(crate::body_codec::pack),
                body.headers,
                body_state.as_str(),
                body.headers_truncated,
                body.encoding_problems,
                body.text.as_deref().map(line_count),
                // Stamped with the parser version, so a later parser can tell which rows it may
                // want back.
                postio_model::mime::PARSER_VERSION,
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        // A body that arrived settles any earlier refusal (#1918).
        self.clear_body_failed(id).await?;
        Ok(())
    }

    /// Keyed to the message's current `MODSEQ`, so a message the server has since changed is no
    /// longer excluded (see [`body_failed`](Self::body_failed)). Only for a refusal about the
    /// message, never a dropped or timed-out connection (#1918).
    pub async fn mark_body_failed(&self, id: MessageId) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO body_failures (message_id, mod_seq, failed_at)
             SELECT id, mod_seq, strftime('%s','now') * 1000 FROM messages WHERE id = ?1
             ON CONFLICT (message_id) DO UPDATE
                SET mod_seq = excluded.mod_seq, failed_at = excluded.failed_at",
            bind![id.get()],
        )
        .await?;
        Ok(())
    }

    pub async fn clear_body_failed(&self, id: MessageId) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM body_failures WHERE message_id = ?1",
            bind![id.get()],
        )
        .await?;
        Ok(())
    }

    /// A new `MODSEQ` is the server saying the message is different, so it is asked again with no
    /// separate clearing. The backfill seeds skip such a message, and the reader says it could not
    /// be downloaded.
    pub async fn body_failed(&self, id: MessageId) -> Result<bool> {
        sql::scalar(
            self.connection,
            "SELECT EXISTS (SELECT 1 FROM body_failures f
                             JOIN messages m ON m.id = f.message_id
                            WHERE f.message_id = ?1 AND f.mod_seq IS m.mod_seq)",
            bind![id.get()],
        )
        .await
        .map(|present| present != 0)
    }

    /// Parser repairs still fetch the source that produced a damaged body.
    pub async fn has_reusable_content(&self, id: MessageId) -> Result<bool> {
        sql::scalar(
            self.connection,
            "SELECT EXISTS (SELECT 1 FROM messages m JOIN message_contents c ON c.id=m.content_id
             WHERE m.id=?1 AND c.namespace IS NOT NULL AND c.body_state IN ('full','partial')
             AND NOT (c.body_encoding_problems=1 AND c.body_parsed_with < ?2))",
            bind![id.get(), postio_model::mime::PARSER_VERSION],
        )
        .await
        .map(|present| present != 0)
    }

    /// Separate from [`set_body`](Self::set_body): reading the blob keys back only to write them
    /// again lets a payload fetch clobber a concurrent text fetch.
    pub async fn set_body_state(&self, id: MessageId, body_state: BodyState) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE message_contents SET body_state = ?2
             WHERE id = (SELECT content_id FROM messages WHERE id = ?1)",
            bind![id.get(), body_state.as_str()],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// `""` says the message is all quote.
    pub async fn set_own_line(&self, id: MessageId, line: &str) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE messages SET own_line = ?2 WHERE id = ?1",
            bind![id.get(), line],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "message",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Keyed by MIME path, not [`AttachmentId`](postio_model::AttachmentId): a refetch replaces a
    /// message's attachment rows, so the id the reading pane held does not survive, but `2.1` stays
    /// `2.1`.
    ///
    /// `false` when no such part is on the message any more.
    pub async fn set_attachment_blob(
        &self,
        message_id: MessageId,
        part_id: &str,
        blob_id: &BlobId,
    ) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE attachments SET blob_id = ?3
              WHERE message_id = ?1 AND part_id = ?2",
            bind![message_id.get(), part_id, blob_id.as_str()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// The body fetch's write in place of [`update`](Self::update), which rewrites every recipient,
    /// attachment and label row (a statement per name, per body of a first sync). A `preview`
    /// already on the row is kept; a `None` raw blob leaves the column as it was.
    pub async fn set_fetched(
        &self,
        id: MessageId,
        preview: Option<&str>,
        raw_blob: Option<&BlobId>,
    ) -> Result<()> {
        sql::execute(
            self.connection,
            "UPDATE message_contents
                SET preview = coalesce(preview, ?2),
                    raw_blob_id = coalesce(?3, raw_blob_id)
              WHERE id = (SELECT content_id FROM messages WHERE id = ?1)",
            bind![id.get(), preview, raw_blob.map(BlobId::as_str)],
        )
        .await?;
        Ok(())
    }

    /// The complement of [`needing_backfill_from`](Self::needing_backfill_from), which stops at
    /// `partial`: text local, payloads not, is settled for the text lane (#376, #377). A part with
    /// no `part_id` is not a candidate: nothing to name in a `FETCH`.
    pub async fn needing_payloads_from(
        &self,
        mailbox_id: MailboxId,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BackfillCandidate>> {
        sql::all(
            self.connection,
            NEEDING_PAYLOADS,
            bind![mailbox_id.get(), limit, offset],
            |row| read_backfill_candidate(row, mailbox_id, role_at(row, 6)?),
        )
        .await
    }

    /// Scoped to one mailbox: seeks `idx_messages_list` and walks its newest rows, never the table.
    /// A message with no `UID` yet is not a candidate.
    pub async fn needing_backfill(
        &self,
        mailbox_id: MailboxId,
        limit: u32,
    ) -> Result<Vec<BackfillCandidate>> {
        self.needing_backfill_from(mailbox_id, limit, 0).await
    }

    /// For walking past a run the scheduler cannot use: a message over `max_body_bytes`, or whose
    /// fetch failed this session, stays `body_state <> 'full'`, so a newest batch of them would
    /// hide the walk for ever (#318). The offset is stable only for one seed; a body landing
    /// between calls shifts it by one, which the next call corrects.
    ///
    /// A body an older parser got wrong is a candidate again: bodies are stamped with
    /// `body_parsed_with` and their raw bytes are not kept, so a row below
    /// [`postio_model::mime::PARSER_VERSION`] that carried the decode caveat is fetched once more.
    /// The fetch stamps the current version whatever it finds, so it is asked for once.
    pub async fn needing_backfill_from(
        &self,
        mailbox_id: MailboxId,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BackfillCandidate>> {
        // Three seeks rather than one `OR`: no index serves a disjunction, so one statement walked
        // the folder and filtered it, passing over every already-fetched body. Each arm seeks an
        // index in `sort_at` order (the folder's list key, so a woken snooze is fetched first) and
        // asks for the whole `offset + limit`, because the merged order decides what is skipped.
        let window = i64::from(limit) + i64::from(offset);

        let mut found: Vec<(i64, BackfillCandidate)> = Vec::new();
        for arm in BACKFILL_ARMS {
            let statement = backfill_arm(arm.predicate);
            let read = |row: &Row| {
                Ok((
                    row.col::<i64>(7)?,
                    read_backfill_candidate(row, mailbox_id, role_at(row, 6)?)?
                        .with_text_size(row.col::<i64>(8)? as u64),
                ))
            };
            let rows = if arm.uses_parser_version {
                let arguments = bind![mailbox_id.get(), window, postio_model::mime::PARSER_VERSION];
                sql::all(self.connection, &statement, arguments, read).await?
            } else {
                let arguments = bind![mailbox_id.get(), window];
                sql::all(self.connection, &statement, arguments, read).await?
            };
            found.extend(rows);
        }

        // A message can be in two arms (owing its body and carrying the caveat): newest first by
        // `sort_at`, ties by id.
        found.sort_by(|(a_at, a), (b_at, b)| {
            (b_at, b.message_id.get()).cmp(&(a_at, a.message_id.get()))
        });
        found.dedup_by_key(|(_, candidate)| candidate.message_id);
        Ok(found
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|(_, candidate)| candidate)
            .collect())
    }

    /// Two reads of rows the store keeps: weights come from `account_footprints`, maintained by the
    /// triggers in `schema/footprint.sql`, and completeness is a question about the account's
    /// folders. It used to be three aggregates over every message (#1508).
    pub async fn footprint(&self, account_id: AccountId) -> Result<StorageFootprint> {
        let (messages, total_bytes, attachment_bytes, local_bytes) = sql::first(
            self.connection,
            "SELECT messages, total_bytes, attachment_bytes, local_bytes
               FROM account_footprints WHERE account_id = ?1",
            [account_id.get()],
            |row| {
                Ok((
                    row.col::<i64>(0)? as u64,
                    row.col::<i64>(1)? as u64,
                    row.col::<i64>(2)? as u64,
                    row.col::<i64>(3)? as u64,
                ))
            },
        )
        .await?
        .unwrap_or_default();

        // Complete when no selectable mailbox awaits its first full header pass; otherwise the
        // totals are a floor.
        let outstanding: i64 = sql::one(
            self.connection,
            "SELECT count(*) FROM mailboxes mb
               LEFT JOIN sync_state s ON s.mailbox_id = mb.id
              WHERE mb.account_id = ?1 AND mb.selectable = 1
                AND (s.last_full_sync_at IS NULL)",
            [account_id.get()],
            |row| row.col(0),
        )
        .await?;

        Ok(StorageFootprint {
            messages,
            total_bytes,
            attachment_bytes,
            local_bytes,
            complete: outstanding == 0,
        })
    }

    /// A count, not a list: Postio never sends a receipt automatically, so the privacy pane has
    /// only the fact of how often one was asked (#970). A row the store keeps
    /// (`schema/footprint.sql`, #1508).
    pub async fn read_receipt_requested_count(&self, account_id: AccountId) -> Result<u64> {
        let counted = sql::first(
            self.connection,
            "SELECT read_receipts FROM account_footprints WHERE account_id = ?1",
            [account_id.get()],
            |row| row.col::<i64>(0),
        )
        .await?;
        Ok(counted.unwrap_or(0) as u64)
    }

    /// For the interactive lane: the reading pane knows only the opened message, so this looks up
    /// its mailbox and server name. `None` covers both "already full" and "gone".
    pub async fn backfill_candidate(
        &self,
        message_id: MessageId,
    ) -> Result<Option<BackfillCandidate>> {
        self.candidate_where(message_id, "messages.body_state <> 'full'")
            .await
    }

    /// The text axis stores no raw source (ADR 0017), so a message with no attachments is `full`
    /// with nothing for view source, export or drag-out (#1984). `None` once the raw blob is
    /// stored, or when the message is gone or has no server identity.
    pub async fn whole_candidate(
        &self,
        message_id: MessageId,
    ) -> Result<Option<BackfillCandidate>> {
        self.candidate_where(message_id, "messages.raw_blob_id IS NULL")
            .await
    }

    async fn candidate_where(
        &self,
        message_id: MessageId,
        condition: &str,
    ) -> Result<Option<BackfillCandidate>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT messages.id, messages.uid, messages.size, messages.received_at,
                    mailboxes.path, messages.remote_id, messages.mailbox_id,
                    mailboxes.role, MAX(0, messages.size - (SELECT COALESCE(SUM(attachments.size), 0) FROM attachments WHERE attachments.message_id = messages.id AND attachments.part_id IS NOT NULL))
               FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
              WHERE messages.id = ?1
                AND {condition}
                AND messages.uid IS NOT NULL
                AND messages.remote_id IS NOT NULL
                AND messages.deleted_locally = 0"),
        )
        .await?;
        crate::sql::first_of(&mut statement, [message_id.get()], |row| {
            let mailbox_id = MailboxId::new(row.col(6)?);
            Ok(read_backfill_candidate(row, mailbox_id, role_at(row, 7)?)?
                .with_text_size(row.col::<i64>(8)? as u64))
        })
        .await
    }

    /// The only messages [`crate::repository::ThreadingRepository::reconsider`] can have a better
    /// answer for later: one naming an ancestor already found it or is waiting to. And only those
    /// whose subject reads as a reply, filtered from two columns rather than loading every
    /// newsletter.
    pub async fn subject_only_orphans(&self, mailbox_id: MailboxId) -> Result<Vec<MessageId>> {
        let mut statement = sql::statement(
            self.connection,
            "SELECT id, subject FROM messages
              WHERE mailbox_id = ?1
                AND thread_id IS NOT NULL
                AND in_reply_to IS NULL
                AND reference_ids = ''",
        )
        .await?;
        let rows: Vec<(i64, Option<String>)> =
            sql::mapped(&mut statement, [mailbox_id.get()], |row| {
                Ok((row.col(0)?, row.col(1)?))
            })
            .await?;
        Ok(rows
            .into_iter()
            .filter(|(_, subject)| {
                subject
                    .as_deref()
                    .is_some_and(postio_model::subject::is_reply)
            })
            .map(|(id, _)| MessageId::new(id))
            .collect())
    }
}

/// `deleted_locally = 0` is here, not in the index: it filters a handful of rows, while the list
/// index gives the order and the scope the range.
/// Every ordinary scope hides a message until its snooze is due, read against wall-clock time on
/// every page (the live half of `mailboxes.snoozed_count`, migration 0021); [`ListScope::Snoozed`]
/// inverts it.
/// The three walks are mailbox-scoped and newest first by `sort_at`, the order `idx_messages_list`
/// holds, so each seeks the folder and stops at its window; a woken snooze is fetched first.
const NEEDING_A_HEADER_FETCH: &str = "\
SELECT messages.id, messages.uid, messages.size, messages.received_at,
       mailboxes.path, messages.remote_id, mailboxes.role
  FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
 WHERE messages.mailbox_id = ?1
   AND messages.body_has_headers = 0
   AND messages.raw_blob_id IS NULL
   AND messages.body_state IN ('partial', 'full')
   AND messages.uid IS NOT NULL
   AND messages.remote_id IS NOT NULL
   AND messages.deleted_locally = 0
 ORDER BY messages.sort_at DESC
 LIMIT ?2";

const NEEDING_PAYLOADS: &str = "\
SELECT messages.id, messages.uid, messages.size, messages.received_at,
       mailboxes.path, messages.remote_id, mailboxes.role
  FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
 WHERE messages.mailbox_id = ?1
   AND messages.body_state = 'partial'
   AND messages.uid IS NOT NULL
   AND messages.remote_id IS NOT NULL
   AND messages.deleted_locally = 0
   AND EXISTS (SELECT 1 FROM attachments
                WHERE attachments.message_id = messages.id
                  AND attachments.blob_id IS NULL
                  AND attachments.part_id IS NOT NULL)
 ORDER BY messages.sort_at DESC
 LIMIT ?2 OFFSET ?3";

struct BackfillArm {
    predicate: &'static str,
    uses_parser_version: bool,
}

/// Three seeks, each into an index holding its rows newest first by `sort_at`
/// (`idx_messages_body_state`, `idx_messages_body_problems`), merged in code.
const BACKFILL_ARMS: [BackfillArm; 3] = [
    BackfillArm {
        predicate: "messages.body_state = 'not_fetched'",
        uses_parser_version: false,
    },
    BackfillArm {
        predicate: "messages.body_state = 'headers_only'",
        uses_parser_version: false,
    },
    BackfillArm {
        predicate: "messages.body_encoding_problems = 1 AND messages.body_parsed_with < ?3",
        uses_parser_version: true,
    },
];

/// `sort_at` is the last column, for the merge.
fn backfill_arm(predicate: &str) -> String {
    format!(
        "SELECT messages.id, messages.uid, messages.size, messages.received_at, \
                mailboxes.path, messages.remote_id, mailboxes.role, messages.sort_at, \
                MAX(0, messages.size - (SELECT COALESCE(SUM(attachments.size), 0) FROM attachments WHERE attachments.message_id = messages.id AND attachments.part_id IS NOT NULL))
           FROM messages JOIN mailboxes ON mailboxes.id = messages.mailbox_id
          WHERE messages.mailbox_id = ?1 AND {predicate}
            AND messages.uid IS NOT NULL
            AND messages.remote_id IS NOT NULL
            AND messages.deleted_locally = 0
            AND NOT EXISTS (SELECT 1 FROM body_failures f
                             WHERE f.message_id = messages.id
                               AND f.mod_seq IS messages.mod_seq)
          ORDER BY messages.sort_at DESC, messages.id DESC
          LIMIT ?2"
    )
}

const NOT_YET_DUE: &str =
    "(messages.snoozed_until IS NULL OR messages.snoozed_until <= (strftime('%s','now') * 1000))";
const STILL_SNOOZED: &str =
    "messages.snoozed_until IS NOT NULL AND messages.snoozed_until > (strftime('%s','now') * 1000)";

fn where_clause(query: &ListQuery, with_cursor: bool) -> String {
    let focus: String;
    let (scope, snooze) = match query.scope {
        // The Drafts exclusion rides on the mailbox scope because a draft's mirror row is in the
        // Drafts folder; the Outbox is a second predicate over it (spec 003).
        // A column comparison, not a subquery, and not a filter above SQL: this clause is paid by
        // every folder on every open, and dropping rows after the fetch breaks paging.
        ListScope::Mailbox(_) => (
            "messages.mailbox_id = ?1 AND (messages.send_state IS NULL \
             OR messages.send_state NOT IN ('queued', 'sending', 'sent'))",
            NOT_YET_DUE,
        ),
        ListScope::Account(_) => ("messages.account_id = ?1", NOT_YET_DUE),
        // Every enabled account's inbox (#1692): nothing to name. The list itself pages
        // conversations through `ThreadRepository::unified_page`; this is the predicate's meaning
        // for a count or flat read.
        ListScope::Unified => (
            "messages.mailbox_id IN (
                 SELECT m.id FROM accounts a JOIN mailboxes m
                     ON m.account_id = a.id AND m.role = 'inbox'
                  WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1)",
            NOT_YET_DUE,
        ),
        ListScope::Flagged(_) => (
            "messages.account_id = ?1 AND messages.flagged = 1",
            NOT_YET_DUE,
        ),
        ListScope::Snoozed(_) => ("messages.account_id = ?1", STILL_SNOOZED),
        // Answered through `idx_messages_send_state` without touching the account's mail.
        // `NOT_YET_DUE` like every ordinary scope: a draft cannot be snoozed, so it costs nothing.
        ListScope::Outbox(_) => (
            "messages.account_id = ?1 AND messages.send_state IN ('queued', 'sending')",
            NOT_YET_DUE,
        ),
        ListScope::Thread(_) => ("messages.thread_id = ?1", NOT_YET_DUE),
        // Focus's inbox, read flat: the unified inboxes less what Focus holds back
        // (`focus_excludes`). The account is a membership test against the enabled ones, which
        // `idx_messages_account_*` seeks once per account.
        ListScope::Focus(postio_model::FocusScope::Flagged) => (
            "messages.account_id IN (SELECT id FROM accounts
                 WHERE enabled = 1 AND pending_deletion = 0) AND messages.flagged = 1",
            NOT_YET_DUE,
        ),
        ListScope::Focus(postio_model::FocusScope::Snoozed) => (
            "messages.account_id IN (SELECT id FROM accounts
                 WHERE enabled = 1 AND pending_deletion = 0)",
            STILL_SNOOZED,
        ),
        ListScope::Focus(_) => {
            focus = format!(
                "messages.mailbox_id IN (
                 SELECT m.id FROM accounts a JOIN mailboxes m
                     ON m.account_id = a.id AND m.role = 'inbox'
                  WHERE a.enabled = 1 AND a.pending_deletion = 0 AND m.selectable = 1){}",
                super::focus_excludes("messages.")
            );
            (focus.as_str(), NOT_YET_DUE)
        }
    };
    // Numbered after the scope's own arguments, so a scope that names nothing leaves no hole at ?1.
    let first = scope_arguments(&query.scope).len() + 1;
    let cursor = if with_cursor {
        // A redundant `<=` ahead of the tie-break, not the row value `(received_at, id) < (?, ?)`:
        // this engine seeks on the scope alone and then filters a row value, so a page 95,000 rows
        // deep walks every row above it (1.4 ms first page, 107 ms deep). The leading `<=` gives
        // the planner a range to seek on; `plan_seeks_past_the_cursor` asserts on the plan.
        let key = order_key(&query.scope);
        format!(
            " AND messages.{key} <= ?{first}
              AND (messages.{key} < ?{first} OR messages.id < ?{})",
            first + 1
        )
    } else {
        String::new()
    };
    format!("{scope} AND messages.deleted_locally = 0 AND {snooze}{cursor}")
}

fn scope_arguments(scope: &ListScope) -> Vec<i64> {
    match scope {
        ListScope::Unified | ListScope::Focus(_) => Vec::new(),
        ListScope::Mailbox(id) => vec![id.get()],
        ListScope::Account(id)
        | ListScope::Flagged(id)
        | ListScope::Snoozed(id)
        | ListScope::Outbox(id) => vec![id.get()],
        ListScope::Thread(id) => vec![id.get()],
    }
}

fn page_arguments(query: &ListQuery) -> Vec<i64> {
    let mut arguments = scope_arguments(&query.scope);
    if let Some(cursor) = query.after {
        arguments.push(to_millis(match query.scope {
            ListScope::Mailbox(_) => cursor.sort_at,
            _ => cursor.received_at,
        }));
        arguments.push(cursor.id.get());
    }
    arguments
}

/// A folder by `sort_at` over `idx_messages_list`, shared with its conversation list so a woken
/// snooze tops both (spec 007, research R7). Query views by `received_at`.
fn order_key(scope: &ListScope) -> &'static str {
    match scope {
        ListScope::Mailbox(_) => "sort_at",
        _ => "received_at",
    }
}

/// Excepted by conversation, not row (ADR 0015 Q3, #468): a list shows one row per thread, so
/// excepting only the id would archive all of a thread but its newest message after `Ctrl+A` and a
/// deselect.
///
/// The `IS NULL` arm keeps it total: `NOT IN` against a set containing NULL is NULL and would drop
/// every unthreaded message.
///
/// The caller binds one argument per exception, from `first` upwards.
fn without_conversations(except: &[MessageId], first: usize) -> String {
    if except.is_empty() {
        return String::new();
    }
    let ids = placeholders(except.len(), first);
    format!(
        " AND messages.id NOT IN ({ids}) \
          AND (messages.thread_id IS NULL \
               OR messages.thread_id NOT IN \
                  (SELECT thread_id FROM messages \
                    WHERE id IN ({ids}) AND thread_id IS NOT NULL))"
    )
}

pub(crate) fn placeholders(count: usize, first: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", index + first))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shared with [`MessageRepository::upsert_batch`], already inside a transaction: SQLite has no
/// nested transactions.
///
/// `sort_at` is kept at least `received_at` rather than written: a resync must not put a woken
/// snooze back, and a draft's row must still rise to the top of Drafts when saved again (spec 007,
/// research R7).
async fn write_update(connection: &Connection, message: &mut Message) -> Result<()> {
    let id = require_persisted(message.id.get(), "message")?;

    // A header refresh carries no body, and a body is immutable for a given
    // message: what is stored stays (#1982).
    if message.sync.body_state == BodyState::HeadersOnly {
        keep_fetched_content(connection, message).await?;
    }

    let changed = sql::execute(
        connection,
        "UPDATE messages
            SET account_id = ?2, mailbox_id = ?3, thread_id = ?4, rfc_message_id = ?5,
                in_reply_to = ?6, reference_ids = ?7, subject = ?8,
                normalized_subject = ?9, date = ?10, received_at = ?11, preview = ?12,
                size = ?13, flags = ?14, seen = ?15, flagged = ?16, answered = ?17,
                draft = ?18, deleted = ?19, has_attachments = ?20, uid = ?21,
                uid_validity = ?22, mod_seq = ?23, remote_id = ?24, body_state = ?25,
                flags_dirty = ?26, has_pending_operations = ?27, deleted_locally = ?28,
                last_synced_at = ?29, raw_blob_id = ?30, content_type = ?31, list_id = ?32,
                text_part_id = ?33, text_part_headers = ?34,
                html_part_id = ?35, html_part_headers = ?36, text_is_flowed = ?37,
                read_receipt_requested = ?38, sort_at = max(sort_at, ?11),
                unsubscribe_offered = coalesce(?39, unsubscribe_offered),
                automation = coalesce(?40, automation),
                content_namespace = ?41, content_key = ?42
          WHERE id = ?1",
        row_values(id, message),
    )
    .await?;
    if changed == 0 {
        return Err(Error::NotFound {
            entity: "message",
            id,
        });
    }

    sql::execute(
        connection,
        "DELETE FROM recipients WHERE message_id = ?1",
        [id],
    )
    .await?;
    sql::execute(
        connection,
        "DELETE FROM attachments WHERE message_id = ?1",
        [id],
    )
    .await?;
    sql::execute(
        connection,
        "DELETE FROM message_labels WHERE message_id = ?1",
        [id],
    )
    .await?;
    write_children(connection, message).await?;
    Ok(())
}

async fn insert(connection: &Connection, message: &Message) -> Result<MessageId> {
    // Cached: the widest statement in the write path, run once per new message of a first sync
    // (#728).
    sql::statement(
        connection,
        "INSERT INTO messages (id, account_id, mailbox_id, thread_id, rfc_message_id,
                               in_reply_to, reference_ids, subject, normalized_subject, date,
                               received_at, preview, size, flags, seen, flagged, answered,
                               draft, deleted, has_attachments, uid, uid_validity, mod_seq,
                               remote_id, body_state, flags_dirty, has_pending_operations,
                               deleted_locally, last_synced_at, raw_blob_id, content_type,
                               list_id, text_part_id, text_part_headers,
                               html_part_id, html_part_headers, text_is_flowed,
                               read_receipt_requested, sort_at, unsubscribe_offered,
                               automation, content_namespace, content_key)
         VALUES (NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                 ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31,
                 ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?11, ?39, ?40, ?41, ?42)",
    )
    .await?
    .execute(row_values(0, message))
    .await?;
    Ok(MessageId::new(connection.last_insert_rowid()))
}

/// Content is immutable for a message on a server, so a flag change, new `MODSEQ` or move says
/// nothing about it: body state, raw blob, preview, part map and attachment rows are carried onto
/// the refreshed message. A real content change is a different row (another UID or `UIDVALIDITY`).
async fn keep_fetched_content(connection: &Connection, message: &mut Message) -> Result<()> {
    let Some(stored) = sql::first(
        connection,
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        [message.id.get()],
        read_message,
    )
    .await?
    else {
        return Ok(());
    };
    if stored.sync.body_state == BodyState::HeadersOnly {
        return Ok(());
    }
    message.sync.body_state = stored.sync.body_state;
    message.raw_blob_id = stored.raw_blob_id;
    message.preview = stored.preview;
    message.content_type = stored.content_type;
    message.text_part_id = stored.text_part_id;
    message.text_part_headers = stored.text_part_headers;
    message.html_part_id = stored.html_part_id;
    message.html_part_headers = stored.html_part_headers;
    message.text_is_flowed = stored.text_is_flowed;
    message.read_receipt_requested = stored.read_receipt_requested;
    message.attachments = read_attachments(connection, message.id).await?;
    Ok(())
}

/// Only for a late header fetch with shared content; ordinary IMAP writes skip the lookup.
async fn reuse_content_projection(connection: &Connection, message: &mut Message) -> Result<()> {
    if message.server.content_identity.is_none() {
        return Ok(());
    }
    sql::execute(
        connection,
        "UPDATE messages SET content_id = content_id WHERE id = ?1",
        [message.id.get()],
    )
    .await?;
    let Some(stored) = sql::first(
        connection,
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        [message.id.get()],
        read_message,
    )
    .await?
    else {
        return Ok(());
    };
    message.sync.body_state = stored.sync.body_state;
    message.raw_blob_id = stored.raw_blob_id;
    message.preview = stored.preview;
    message.content_type = stored.content_type;
    message.text_part_id = stored.text_part_id;
    message.text_part_headers = stored.text_part_headers;
    message.html_part_id = stored.html_part_id;
    message.html_part_headers = stored.html_part_headers;
    message.text_is_flowed = stored.text_is_flowed;
    message.read_receipt_requested = stored.read_receipt_requested;
    message.attachments = read_attachments(connection, message.id).await?;
    Ok(())
}

/// Owned [`turso::Value`]s: thirty parameters of six types stay aligned with the column list in one
/// homogeneous list.
fn row_values(id: i64, message: &Message) -> Vec<turso::Value> {
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
        // A row the server has not named yet is stored folded so `find_awaiting_identity` can seek
        // it by binary equality; every other row keeps its arrival case, since a reply quotes it
        // byte-for-byte.
        maybe_text(message.rfc_message_id.as_ref().map(|id| {
            if message.server.remote_id.is_none() && message.server.uid.is_none() {
                id.folded()
            } else {
                id.as_str().to_owned()
            }
        })),
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
        maybe_text(
            message
                .server
                .remote_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
        ),
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
        maybe_text(message.text_part_id.clone()),
        maybe_text(message.text_part_headers.clone()),
        maybe_text(message.html_part_id.clone()),
        maybe_text(message.html_part_headers.clone()),
        boolean(message.text_is_flowed),
        boolean(message.read_receipt_requested),
        // NULL while unknown, which `write_update` reads as "keep what is there".
        maybe_integer(
            message
                .promoted
                .map(|promoted| i64::from(promoted.unsubscribe_offered)),
        ),
        maybe_integer(
            message
                .promoted
                .map(|promoted| i64::from(promoted.automation)),
        ),
        maybe_text(
            message
                .server
                .content_identity
                .as_ref()
                .map(|identity| identity.namespace.clone()),
        ),
        maybe_text(
            message
                .server
                .content_identity
                .as_ref()
                .map(|identity| identity.key.clone()),
        ),
    ]
}

fn integer(value: i64) -> turso::Value {
    turso::Value::Integer(value)
}

fn maybe_integer(value: Option<i64>) -> turso::Value {
    value.map_or(turso::Value::Null, turso::Value::Integer)
}

fn text(value: impl Into<String>) -> turso::Value {
    turso::Value::Text(value.into())
}

fn maybe_text(value: Option<String>) -> turso::Value {
    value.map_or(turso::Value::Null, turso::Value::Text)
}

fn boolean(value: bool) -> turso::Value {
    turso::Value::Integer(i64::from(value))
}

async fn write_children(connection: &Connection, message: &mut Message) -> Result<()> {
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
            sql::execute(
                connection,
                "INSERT INTO recipients (message_id, kind, position, name, address_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                bind![
                    id,
                    kind,
                    position as i64,
                    address.name,
                    address_id(connection, address).await?,
                ],
            )
            .await?;
        }
    }

    for (position, attachment) in message.attachments.iter_mut().enumerate() {
        sql::execute(
            connection,
            "INSERT INTO attachments (message_id, position, filename, mime_type, size,
                                      content_id, disposition, disposition_raw, part_id,
                                      part_headers, blob_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            bind![
                id,
                position as i64,
                attachment.filename,
                attachment.mime_type,
                attachment.size as i64,
                attachment.content_id,
                attachment.disposition.as_str(),
                attachment.disposition.raw(),
                attachment.part_id,
                attachment.part_headers,
                attachment.blob_id.as_ref().map(BlobId::as_str),
            ],
        )
        .await?;
        attachment.id = postio_model::AttachmentId::new(connection.last_insert_rowid());
        attachment.message_id = message.id;
    }

    for label in &message.labels {
        sql::execute(
            connection,
            "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            bind![id, label.get()],
        )
        .await?;
    }

    reuse_content_projection(connection, message).await?;
    Ok(())
}

async fn find_by_remote_id(
    connection: &Connection,
    mailbox_id: MailboxId,
    remote_id: &RemoteId,
) -> Result<Option<MessageId>> {
    // Cached: once per message on every batch a sync pass writes (#728).
    let mut statement = sql::statement(
        connection,
        "SELECT id FROM messages WHERE mailbox_id = ?1 AND remote_id = ?2",
    )
    .await?;
    crate::sql::first_of(
        &mut statement,
        bind![mailbox_id.get(), remote_id.as_str()],
        |row| Ok(MessageId::new(row.col(0)?)),
    )
    .await
}

const ID_LOOKUP_CHUNK: usize = 500;

/// For IMAP a [`Generation`] is numerically the stored `uid_validity` (#543).
async fn find_by_generation_uid(
    connection: &Connection,
    mailbox_id: MailboxId,
    generation: Generation,
    uid: Uid,
) -> Result<Option<MessageId>> {
    // Cached: the fallback lookup runs per message for rows synced before `remote_id` existed.
    let mut statement = sql::statement(
        connection,
        "SELECT id FROM messages
          WHERE mailbox_id = ?1 AND uid_validity = ?2 AND uid = ?3",
    )
    .await?;
    crate::sql::first_of(
        &mut statement,
        bind![
            mailbox_id.get(),
            i64::from(generation.get()),
            i64::from(uid.get()),
        ],
        |row| Ok(MessageId::new(row.col(0)?)),
    )
    .await
}

/// The mailbox role at `index`, for the queries that carry one.
fn role_at(row: &Row, index: usize) -> Result<MailboxRole> {
    let role: String = row.col(index)?;
    MailboxRole::from_name(&role).ok_or_else(|| unknown_enum("mailboxes.role", role))
}

/// `mailbox_role` is passed in: four queries share this reader and disagree on column layout.
fn read_backfill_candidate(
    row: &Row,
    mailbox_id: MailboxId,
    mailbox_role: MailboxRole,
) -> Result<BackfillCandidate> {
    Ok(BackfillCandidate {
        message_id: MessageId::new(row.col(0)?),
        mailbox_id,
        uid: Uid::new(row.col::<i64>(1)? as u32),
        size: row.col::<i64>(2)? as u64,
        // The whole message until a query that knows better says so.
        text_size: row.col::<i64>(2)? as u64,
        received_at: from_millis(row.col(3)?),
        mailbox_path: row.col(4)?,
        remote_id: RemoteId::new(row.col::<String>(5)?),
        // So the backfill queue, one heap across every mailbox, can rank a body by folder: by date
        // alone a large Archive outranks the inbox it was seeded behind.
        mailbox_role,
    })
}

fn read_message(row: &Row) -> Result<Message> {
    let body_state: String = row.col(18)?;
    let references: String = row.col(6)?;

    Ok(Message {
        id: MessageId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        mailbox_id: MailboxId::new(row.col(2)?),
        thread_id: row.col::<Option<i64>>(3)?.map(ThreadId::new),
        rfc_message_id: row.col::<Option<String>>(4)?.map(RfcMessageId::new),
        in_reply_to: row.col::<Option<String>>(5)?.map(RfcMessageId::new),
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
        subject: row.col(7)?,
        date: row.col::<Option<i64>>(8)?.map(from_millis),
        received_at: from_millis(row.col(9)?),
        body: postio_model::MessageBody::default(),
        preview: row.col(10)?,
        attachments: Vec::new(),
        flags: parse_flags(&row.col::<String>(12)?),
        labels: Vec::new(),
        size: row.col::<i64>(11)? as u64,
        headers: postio_model::Headers::new(),
        server: ServerIdentifiers {
            content_identity: match (
                row.col::<Option<String>>(35)?,
                row.col::<Option<String>>(36)?,
            ) {
                (Some(namespace), Some(key)) => {
                    Some(postio_model::ContentIdentity::new(namespace, key))
                }
                _ => None,
            },
            uid: row.col::<Option<i64>>(14)?.map(|uid| Uid::new(uid as u32)),
            uid_validity: row
                .col::<Option<i64>>(15)?
                .map(|validity| UidValidity::new(validity as u32)),
            mod_seq: row
                .col::<Option<i64>>(16)?
                .map(|seq| ModSeq::new(seq as u64)),
            remote_id: row.col::<Option<String>>(17)?.map(RemoteId::new),
        },
        sync: LocalSyncState {
            body_state: BodyState::from_name(&body_state)
                .ok_or_else(|| unknown_enum("messages.body_state", body_state))?,
            flags_dirty: row.col(19)?,
            has_pending_operations: row.col(20)?,
            deleted_locally: row.col(21)?,
            last_synced_at: row.col::<Option<i64>>(22)?.map(from_millis),
        },
        raw_blob_id: row.col::<Option<String>>(23)?.map(BlobId::new),
        content_type: row.col(24)?,
        list_id: row.col(25)?,
        text_part_id: row.col(26)?,
        text_part_headers: row.col(27)?,
        html_part_id: row.col(28)?,
        html_part_headers: row.col(29)?,
        snoozed_until: row.col::<Option<i64>>(30)?.map(from_millis),
        text_is_flowed: row.col(31)?,
        read_receipt_requested: row.col(32)?,
        promoted: match (row.col::<Option<bool>>(33)?, row.col::<Option<i64>>(34)?) {
            (Some(unsubscribe_offered), Some(automation)) => Some(PromotedHeaders {
                unsubscribe_offered,
                automation: u8::try_from(automation).unwrap_or_default(),
            }),
            _ => None,
        },
    })
}

pub(crate) fn read_list_row(row: &Row) -> Result<MessageListRow> {
    read_list_row_at(row, 0)
}

pub(crate) fn read_list_row_at(row: &Row, first: usize) -> Result<MessageListRow> {
    let from_address: Option<String> = row.col(first + 14)?;
    Ok(MessageListRow {
        id: MessageId::new(row.col(first)?),
        thread_id: row.col::<Option<i64>>(first + 1)?.map(ThreadId::new),
        from: from_address
            .map(|address| {
                Ok::<_, Error>(EmailAddress::new(
                    row.col::<Option<String>>(first + 13)?,
                    address,
                ))
            })
            .transpose()?,
        subject: row.col(first + 2)?,
        preview: row.col(first + 3)?,
        received_at: from_millis(row.col(first + 4)?),
        seen: row.col(first + 5)?,
        flagged: row.col(first + 6)?,
        answered: row.col(first + 7)?,
        send_state: row
            .col::<Option<String>>(first + 11)?
            .as_deref()
            .and_then(DraftState::from_name),
        send_at: row.col::<Option<i64>>(first + 12)?.map(from_millis),
        has_attachments: row.col(first + 9)?,
        size: row.col::<i64>(first + 10)? as u64,
        thread_count: row
            .col::<Option<i64>>(first + 15)?
            .map(|count| count.max(0) as u32),
        sort_at: from_millis(row.col(first + 16)?),
    })
}

/// Keyed on the normalized form, so `Ada@Example.com` and `ada@example.com` are one row (migration
/// 0011).
///
/// `ON CONFLICT DO NOTHING` then `SELECT`, not `INSERT OR REPLACE`: replacing would churn an id
/// hundreds of recipients point at.
pub(crate) async fn address_id(connection: &Connection, address: &EmailAddress) -> Result<i64> {
    let normalized = address.normalized();
    sql::execute(
        connection,
        "INSERT INTO addresses (address, address_normalized) VALUES (?1, ?2)
         ON CONFLICT (address_normalized) DO NOTHING",
        bind![address.address, normalized],
    )
    .await?;
    sql::one(
        connection,
        "SELECT id FROM addresses WHERE address_normalized = ?1",
        bind![normalized],
        |row| row.col(0),
    )
    .await
}

async fn read_recipients(connection: &Connection, message: &mut Message) -> Result<()> {
    let mut statement = sql::statement(
        connection,
        "SELECT r.kind, r.name, a.address FROM recipients r
           JOIN addresses a ON a.id = r.address_id
          WHERE r.message_id = ?1 ORDER BY r.kind, r.position, r.id",
    )
    .await?;
    let rows = sql::mapped(&mut statement, [message.id.get()], |row| {
        Ok((
            row.col::<String>(0)?,
            EmailAddress::new(row.col::<Option<String>>(1)?, row.col::<String>(2)?),
        ))
    })
    .await?;

    for (kind, address) in rows {
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

async fn read_attachments(connection: &Connection, id: MessageId) -> Result<Vec<Attachment>> {
    sql::all(
        connection,
        "SELECT id, filename, mime_type, size, content_id, disposition, disposition_raw,
                part_id, blob_id, part_headers
           FROM attachments WHERE message_id = ?1 ORDER BY position, id",
        [id.get()],
        |row| {
            let disposition: String = row.col(5)?;
            let raw: Option<String> = row.col(6)?;
            Ok(Attachment {
                id: postio_model::AttachmentId::new(row.col(0)?),
                message_id: id,
                filename: row.col(1)?,
                mime_type: row.col(2)?,
                size: row.col::<i64>(3)? as u64,
                content_id: row.col(4)?,
                disposition: Disposition::from_parts(&disposition, raw.as_deref())
                    .ok_or_else(|| unknown_enum("attachments.disposition", disposition))?,
                part_id: row.col(7)?,
                blob_id: row.col::<Option<String>>(8)?.map(BlobId::new),
                part_headers: row.col(9)?,
            })
        },
    )
    .await
}

async fn read_labels(connection: &Connection, id: MessageId) -> Result<Vec<LabelId>> {
    sql::all(
        connection,
        "SELECT label_id FROM message_labels WHERE message_id = ?1 ORDER BY label_id",
        [id.get()],
        |row| Ok(LabelId::new(row.col(0)?)),
    )
    .await
}

/// `seen` and `flagged` are SQL expressions so a bulk write can substitute the value it sets for
/// the one the column still holds; see [`MessageRepository::set_flag_on_set`].
fn flags_expression(seen: &str, flagged: &str) -> String {
    let system = [
        (Flag::Seen.as_str(), seen.to_owned()),
        (Flag::Answered.as_str(), "messages.answered".to_owned()),
        (Flag::Flagged.as_str(), flagged.to_owned()),
        (Flag::Deleted.as_str(), "messages.deleted".to_owned()),
        (Flag::Draft.as_str(), "messages.draft".to_owned()),
    ];
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

/// `(mailbox, remote_id)`, scoped to the account's Drafts mailbox because IMAP identities are
/// per-mailbox. A draft whose append the server would not locate has no identity and is absent:
/// `postio-sync` flags the folder for a resync instead of guessing.
async fn own_draft_copies(connection: &Connection) -> Result<BTreeSet<(MailboxId, String)>> {
    let mut statement = sql::statement(
        connection,
        "SELECT mailboxes.id, drafts.remote_id
           FROM drafts
           JOIN mailboxes ON mailboxes.account_id = drafts.account_id
                         AND mailboxes.role = 'drafts'
          WHERE drafts.remote_id IS NOT NULL",
    )
    .await?;
    let rows = sql::mapped(&mut statement, (), |row| {
        Ok((MailboxId::new(row.col::<i64>(0)?), row.col(1)?))
    })
    .await?;
    Ok::<_, Error>(rows.into_iter().collect())
}

/// Only a row with no server identity at all: Postio files a sent message in Sent before the
/// `APPEND` that names it (#942), or when that append fails. Without this a resync inserted the
/// server's copy as a second row and Sent showed the message twice.
///
/// Narrow on purpose: a row that carries an identity is a different message sharing a `Message-ID`
/// (a mailing list's copy of one's own post), and collapsing them would lose one.
///
/// One indexed seek per message that missed both identity routes, not a scan per batch (the scan
/// cost an hour of CPU on a first sync); no partial index can help, as this planner will not read
/// through one (ADR 0017).
///
/// Compared against the folded key, binary, because `COLLATE NOCASE` would make the index unusable:
/// [`row_values`] stores a row with no server identity folded ([`RfcMessageId::folded`]).
async fn find_awaiting_identity(
    connection: &Connection,
    account_id: AccountId,
    mailbox_id: MailboxId,
    rfc: &RfcMessageId,
) -> Result<Option<MessageId>> {
    let mut statement = sql::statement(
        connection,
        "SELECT id FROM messages
          WHERE account_id = ?1 AND rfc_message_id = ?2 AND mailbox_id = ?3
            AND remote_id IS NULL
            AND uid IS NULL
            AND deleted_locally = 0",
    )
    .await?;
    crate::sql::first_of(
        &mut statement,
        bind![account_id.get(), rfc.folded(), mailbox_id.get()],
        |row| Ok(MessageId::new(row.col(0)?)),
    )
    .await
}

/// The header-less twin of [`find_awaiting_identity`] (#1994): matched on the internal date and
/// size, which the server copies verbatim into the destination. Used only when the server's copy
/// has no `Message-ID` either, and only against rows with no identity at all. Two header-less rows
/// with the same date and size are interchangeable, so the first is taken. One seek on
/// `idx_messages_account_list`, then a filter over the few rows at that instant.
///
/// A draft's own row is never a candidate: an unlocated append is resolved by a resync of Drafts.
async fn find_awaiting_headerless(
    connection: &Connection,
    account_id: AccountId,
    mailbox_id: MailboxId,
    received_at: DateTime<Utc>,
    size: u64,
) -> Result<Option<MessageId>> {
    let mut statement = sql::statement(
        connection,
        "SELECT id FROM messages
          WHERE account_id = ?1 AND received_at = ?2
            AND mailbox_id = ?3 AND size = ?4
            AND rfc_message_id IS NULL
            AND remote_id IS NULL
            AND uid IS NULL
            AND deleted_locally = 0
            AND NOT EXISTS (SELECT 1 FROM drafts WHERE drafts.message_id = messages.id)
          ORDER BY id
          LIMIT 1",
    )
    .await?;
    crate::sql::first_of(
        &mut statement,
        bind![
            account_id.get(),
            to_millis(received_at),
            mailbox_id.get(),
            size as i64
        ],
        |row| Ok(MessageId::new(row.col(0)?)),
    )
    .await
}

/// Only `move` and `delete` (their queue row names the mailbox the message is leaving) and only
/// `pending` or `in_flight`: `done` means the server stops listing it, and a shadow that outlived a
/// `failed` one would hide the message for ever. See [`MessageRepository::upsert_batch`] and #368.
///
/// Keyed on the queue row's snapshot, because the local half of the move nulls the row's
/// `uid`/`uid_validity` in the same transaction (#289).
async fn shadowed_by_pending_operation(
    connection: &Connection,
) -> Result<BTreeSet<(MailboxId, String)>> {
    let mut statement = sql::statement(
        connection,
        "SELECT mailbox_id, source_remote_id
           FROM operation_queue
          WHERE state IN ('pending', 'in_flight')
            AND op_type IN ('move', 'delete')
            AND mailbox_id IS NOT NULL
            AND source_remote_id IS NOT NULL",
    )
    .await?;
    let rows = sql::mapped(&mut statement, (), |row| {
        Ok((MailboxId::new(row.col::<i64>(0)?), row.col(1)?))
    })
    .await?;
    Ok::<_, Error>(rows.into_iter().collect())
}

/// A merge, not a skip (#317): unlike a move, the row is still there and its subject, size and
/// parts are worth taking; only the flags the queue holds intent about must survive. Keyed by local
/// message id, since a flag operation names its target directly.
async fn unacknowledged_flag_changes(
    connection: &Connection,
) -> Result<BTreeMap<MessageId, Vec<(bool, FlagSet)>>> {
    let mut statement = sql::statement(
        connection,
        "SELECT target_id, op_type, payload
           FROM operation_queue
          WHERE state IN ('pending', 'in_flight')
            AND op_type IN ('set_flags', 'clear_flags')
            AND target_kind = 'message'
          ORDER BY id",
    )
    .await?;
    let rows = sql::mapped(&mut statement, (), |row| {
        Ok((
            row.col::<i64>(0)?,
            row.col::<String>(1)?,
            row.col::<String>(2)?,
        ))
    })
    .await?;

    let mut changes: BTreeMap<MessageId, Vec<(bool, FlagSet)>> = BTreeMap::new();
    for (target, op_type, payload) in rows {
        let operation: postio_model::Operation =
            serde_json::from_str(&payload).map_err(|source| Error::CorruptPayload {
                column: "operation_queue.payload",
                source,
            })?;
        let entry = match (&operation, op_type.as_str()) {
            (postio_model::Operation::SetFlags { flags }, _) => (true, flags.clone()),
            (postio_model::Operation::ClearFlags { flags }, _) => (false, flags.clone()),
            // The `op_type` filter should make this unreachable; a row whose type and payload
            // disagree is corrupt, and dropping it leaves the server authoritative.
            _ => continue,
        };
        changes
            .entry(MessageId::new(target))
            .or_default()
            .push(entry);
    }
    Ok(changes)
}

/// The server's copy is the base (it carries what the user did not touch, including flags from
/// another client), with the queue's unacknowledged intent on top in the order it was made.
fn with_unacknowledged(server: &FlagSet, changes: &[(bool, FlagSet)]) -> FlagSet {
    let mut flags = server.clone();
    for (set, changed) in changes {
        for flag in changed.iter() {
            if *set {
                flags.insert(flag.clone());
            } else {
                flags.remove(flag);
            }
        }
    }
    flags
}

fn flag_text(flags: &FlagSet) -> String {
    flags.iter().map(Flag::as_str).collect::<Vec<_>>().join(" ")
}

fn parse_flags(text: &str) -> FlagSet {
    text.split_whitespace().map(Flag::parse).collect()
}

/// Counted when the body is written, never from laid-out output (unavailable before the work the
/// rail exists to skip, and width-dependent) (#1329). Counted on the plain text, not markup. A
/// trailing newline does not add a line.
fn line_count(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    text.lines().count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    /// The header-fetch walk seeks `idx_messages_list`; the payload walk and the bodies walk's
    /// three arms each seek a `sort_at`-ordered index. No statement gathers the folder's candidates
    /// to sort them (a folder of 60,000, for fifty).
    #[tokio::test]
    async fn the_backfill_s_folder_walks_seek_an_index_and_never_sort() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let mut walks: Vec<(String, String, &str)> = vec![
            (
                "a header fetch".into(),
                NEEDING_A_HEADER_FETCH.into(),
                "idx_messages_list",
            ),
            (
                "payloads".into(),
                NEEDING_PAYLOADS.into(),
                "idx_messages_body_state",
            ),
        ];
        for (arm, index) in BACKFILL_ARMS.iter().zip(
            ["idx_messages_body_state"; 2]
                .into_iter()
                .chain(["idx_messages_body_problems"]),
        ) {
            walks.push((
                format!("bodies ({})", arm.predicate),
                backfill_arm(arm.predicate),
                index,
            ));
        }
        for (walk, sql, index) in walks {
            let plan = test_support::plan(&connection, &sql).await;
            assert!(
                plan.contains(index) && !test_support::sorts(&plan),
                "{walk}: the walk sorts rather than seeking {index}:\n{plan}"
            );
        }
    }
}
