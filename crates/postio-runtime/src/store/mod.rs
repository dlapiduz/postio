//! Reading the local store, off whatever thread asked.
//!
//! # Why this is here and not in the frontend
//!
//! `postio-gtk` must not depend on `rusqlite` — CI enforces it — so the view
//! layer cannot read `postio-storage` itself. It also must never *wait* on a
//! read: every widget is main-thread only, and a query that blocked the main
//! loop would cost frames on the one interaction that happens most.
//!
//! `Store` is both halves of that answer. It owns the connection pool, runs
//! every query on a blocking thread through `tokio::task::spawn_blocking`, and
//! hands back types built out of [`postio_model`] — which the frontend already
//! depends on — rather than anything of `postio-storage`'s. The rows a
//! frontend sees have no SQL in their ancestry, which is what keeps a second
//! frontend possible.
//!
//! # Why the count travels with the page
//!
//! [`MessagePage`] carries `total` alongside its rows because they come from
//! one read. Asked separately they can disagree — mail arrives between the two
//! — and a list told it is 900 rows long that only ever receives 899 is a list
//! with a permanent gap at the bottom.
//!
//! # What it costs
//!
//! One `spawn_blocking` per call, which is a pool thread and not a tokio
//! worker, so a slow query delays no other task. `Pool` hands each of those
//! threads its own connection rather than serialising them behind one mutex.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use postio_model::address::EmailAddress;
use postio_model::ids::{AccountId, MessageId, ThreadId};
use postio_model::mailbox::Mailbox;

/// Which messages the list is showing.
///
/// `postio-model`'s: every reader of the list, from `postio-storage`'s own
/// queries to a frontend's feed, answers this the same way (#670).
pub use postio_model::ListScope;

/// Which rows are wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    /// The messages being listed.
    pub scope: ListScope,
    /// The first row wanted, counted from the newest.
    pub offset: u32,
    /// How many rows to read.
    pub limit: u32,
}

/// One row of the message list, as a frontend needs it.
///
/// `postio-storage` has a struct with nearly these fields; this one adds the
/// thread count the row's badge shows and drops the size, which nothing draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSummary {
    /// Local id. The row's identity, across reloads and updates.
    pub id: MessageId,
    /// The thread it belongs to, for drill-in and the count badge.
    pub thread: Option<ThreadId>,
    /// Who it is from.
    pub from: Option<EmailAddress>,
    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// The snippet under the subject.
    pub preview: Option<String>,
    /// When the server received it; the list's sort key.
    pub received_at: DateTime<Utc>,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it carries `\Flagged`.
    pub flagged: bool,
    /// Whether it has been replied to.
    pub answered: bool,
    /// Whether it is a draft.
    pub draft: bool,
    /// Whether it has an attachment.
    pub has_attachments: bool,
    /// How many messages are in its thread; the badge appears above one.
    pub thread_count: u32,
}

/// One row of the threaded message list, as a frontend needs it.
///
/// A folder shows one row per conversation (ADR 0015), and this is that row.
/// Three of its numbers are **scoped to the folder that was asked for** and
/// one deliberately is not — see the fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    /// The conversation this row stands for.
    ///
    /// `None` for a message belonging to no thread. Threading runs on
    /// everything sync files and can still fail, and a list that hides mail
    /// because a derived column is null is worse than one that shows it
    /// ungrouped — so an unthreaded message is a conversation of one.
    pub id: Option<ThreadId>,
    /// The newest message of the conversation **in this folder**.
    ///
    /// What the row is drawn from and what opening the row reads, so every
    /// surface that already takes a message — the reading pane, drag-out,
    /// reply — keeps working without knowing this is a thread row. A reply
    /// filed in Archive is not what the Inbox row should be showing.
    pub representative: MessageSummary,
    /// The conversation's subject: the normalised root subject.
    pub subject: Option<String>,
    /// Everyone who has written in it, in first-seen order.
    ///
    /// The row elides them for width; it is given all of them because which
    /// ones survive eliding is a drawing decision, not a storage one.
    pub participants: Vec<EmailAddress>,
    /// How many messages the conversation holds, across every folder.
    ///
    /// **Not scoped.** The badge means how big the conversation is, wherever
    /// it is filed (ADR 0015 Q2).
    pub message_count: u32,
    /// How many of its messages are unread **in this folder**.
    ///
    /// Scoped, so a conversation whose only unread member is filed elsewhere
    /// reads as handled here — unread is what you act on from this folder.
    pub unread_count: u32,
    /// Whether any member **in this folder** is flagged.
    pub flagged: bool,
    /// Whether any member has an attachment.
    pub has_attachments: bool,
    /// When the conversation last moved; the list's sort key.
    pub last_at: DateTime<Utc>,
}

impl ThreadSummary {
    /// Whether anything in this folder's slice is unread.
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }
}

/// One page of a folder's conversations, and how many there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPage {
    /// How many conversations the scope matches, as of this read.
    pub total: u32,
    /// The rows themselves, most recently active first.
    pub rows: Vec<ThreadSummary>,
}

/// A page of the list, drawn however its scope lists itself.
///
/// The frontend asks for "the next page of this scope" and draws what comes
/// back. Which of the two windows answers is the **store's** decision, not the
/// caller's, because it depends on what the folder is — and a frontend that
/// had to know would be a second place for ADR 0015's folders-thread /
/// views-list line to be got wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListPage {
    /// The scope lists messages: a query view, or a folder of drafts.
    Messages(MessagePage),
    /// The scope lists conversations: an ordinary folder.
    Threads(ThreadPage),
}

impl ListPage {
    /// How many rows the scope matches, as of this read.
    pub fn total(&self) -> u32 {
        match self {
            ListPage::Messages(page) => page.total,
            ListPage::Threads(page) => page.total,
        }
    }
}

/// One page of a mailbox, and how long the list is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePage {
    /// How many rows the scope matches, as of this read.
    pub total: u32,
    /// The rows themselves, newest first.
    pub rows: Vec<MessageSummary>,
}

/// A read that could not be answered.
///
/// Carries a sentence rather than a storage error, for the same reason the
/// rows do: whatever is on the other side of this boundary must not need to
/// know what SQLite is. The sentence is the one the user should see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    message: String,
}

impl StoreError {
    /// The failure, phrased for the user. Never contains a secret.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

impl StoreError {
    /// A failure phrased for the user.
    pub fn new(message: impl Into<String>) -> Self {
        StoreError {
            message: message.into(),
        }
    }
}

/// The answer to a read, awaited by whoever asked for it.
///
/// Boxed rather than an `async fn` in the trait, because a frontend holds this
/// as a trait object — one store, chosen once, behind a `dyn` — and `async fn`
/// in traits is not object-safe.
pub type Read<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Everything a frontend needs to read out of the local store.
///
/// A trait rather than a struct so the thing that owns a database and the
/// thing that draws its rows need not be compiled together. `postio-gtk`
/// depends on `postio-core`, so anything concrete here would put `rusqlite` in
/// the view layer's dependency graph — which
/// `scripts/checks/check-crate-boundaries.py` refuses, and rightly: the view layer
/// does no SQL. The implementation lives behind the `runtime` feature, and a
/// test can answer from a table instead.
///
/// Every method returns a future rather than a value: reads happen on a
/// blocking thread and the caller awaits, so no UI thread ever waits on
/// SQLite.
pub trait MailStore: Send + Sync {
    /// One page of the message list, with the count that page was read
    /// against.
    fn message_page(&self, request: PageRequest) -> Read<'_, MessagePage>;

    /// One page of the list, however this scope lists itself.
    ///
    /// What the frontend calls. [`MailStore::message_page`] and
    /// [`MailStore::thread_page`] are the two windows underneath it, and are
    /// worth asking for directly only when the caller genuinely means one of
    /// them.
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage>;

    /// How many rows the list would show, however this scope lists itself.
    fn list_count(&self, scope: ListScope) -> Read<'_, u32>;

    /// One page of the *threaded* list, with the count it was read against.
    ///
    /// A real folder threads and a query view does not (ADR 0015), so this
    /// answers only [`ListScope::Mailbox`] and [`ListScope::Account`];
    /// anything else is a caller asking the wrong question and comes back as
    /// an error rather than as message rows wearing a hat.
    fn thread_page(&self, request: PageRequest) -> Read<'_, ThreadPage>;

    /// How many conversations the threaded list would show.
    fn thread_count(&self, scope: ListScope) -> Read<'_, u32>;

    /// How many rows the list would show, without reading any of them.
    fn message_count(&self, scope: ListScope) -> Read<'_, u32>;

    /// The rows for an explicit, ranked set of ids, in the order given.
    ///
    /// For search hits, which no [`ListScope`] describes: they are ranked
    /// rather than sorted, they can span folders, and there is no offset to
    /// page by because the ids are the answer. The caller pages by slicing
    /// the ids and asking for one slice at a time, so this stays as windowed
    /// as the mailbox read.
    ///
    /// No total comes back, because the caller already knows it: the length
    /// of the id list it holds. Ids the store no longer knows about are
    /// dropped, so the answer may be shorter than the request.
    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>>;

    /// An account's folders, with their counts as of now.
    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>>;
}

mod sqlite;
pub use sqlite::SqliteStore;
