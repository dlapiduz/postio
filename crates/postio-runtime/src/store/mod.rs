//! Reading the local store, off whatever thread asked.
//!
//! # Why this is here and not in the frontend
//!
//! `postio-gtk` must not depend on the database engine — CI enforces it — so
//! the view layer cannot read `postio-storage` itself. It also must never
//! *wait* on a read: every widget is main-thread only, and a query that
//! blocked the main loop would cost frames on the one interaction that happens
//! most.
//!
//! `Store` is both halves of that answer. It reads through `postio-storage`'s
//! async engine — a plain `await`, no thread pool — and hands back types built
//! out of [`postio_model`], which the frontend already depends on, rather than
//! anything of `postio-storage`'s. The rows a frontend sees have no SQL in
//! their ancestry, which is what keeps a second frontend possible.
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
//! A `connect().await` and the read, both on the caller's task. The engine is
//! async to the bottom (specs/004-turso-store), so there is no blocking
//! thread and no pool: a read is a future like any other, and a slow one
//! yields rather than tying up a worker. This was a `spawn_blocking` onto a
//! pool of connections while the store was SQLite; the swap deleted both.

use std::future::Future;
use std::pin::Pin;

use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::Mailbox;

/// Which messages the list is showing.
///
/// `postio-model`'s: every reader of the list, from `postio-storage`'s own
/// queries to a frontend's feed, answers this the same way (#670).
pub use postio_model::ListScope;

pub use postio_model::listing::{
    ListPage, ListRows, MessagePage, MessageSummary, PageRequest, StoreError, ThreadPage,
    ThreadSummary,
};

/// The answer to a read, awaited by whoever asked for it.
///
/// Boxed rather than an `async fn` in the trait, because a frontend holds this
/// as a trait object — one store, chosen once, behind a `dyn` — and `async fn`
/// in traits is not object-safe.
pub type Read<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Everything a frontend needs to read out of the local store — and only
/// that.
///
/// A trait rather than a struct so the thing that owns a database and the
/// thing that draws its rows need not be compiled together. `postio-gtk`
/// depends on `postio-core`, so anything concrete here would put the
/// database engine in the view layer's dependency graph — which
/// `scripts/checks/check-crate-boundaries.py` refuses, and rightly: the view
/// layer does no SQL. The implementation lives behind the `runtime` feature,
/// and a test can answer from a table instead.
///
/// Five methods, each one something a frontend calls. Which window a scope
/// lists itself as — threaded or flat (ADR 0015) — is the store's decision,
/// answered inside [`list_page`](Self::list_page); the two windows underneath
/// it are [`LocalStore`]'s own methods, for the tests and benches that mean
/// one of them specifically, and are deliberately not part of this contract.
///
/// Every method returns a future rather than a value: reads happen on the
/// runtime and the caller awaits, so no UI thread ever waits on the store.
pub trait MailStore: Send + Sync {
    /// One page of the list, however this scope lists itself.
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage>;

    /// How many rows the list would show, however this scope lists itself.
    fn list_count(&self, scope: ListScope) -> Read<'_, u32>;

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

    /// The rows `scope`'s list shows for these messages, in the shape its
    /// pages use, so a change that names ids can be patched into the rows
    /// on screen rather than answered by re-reading their page (#1607).
    ///
    /// A store that can only read pages answers with an error, and the list
    /// falls back to the page; that is what this default is, so a fake in a
    /// test does not have to say so.
    fn rows_in(&self, scope: ListScope, ids: Vec<MessageId>) -> Read<'_, ListRows> {
        let _ = (scope, ids);
        Box::pin(async { Err(StoreError::new("this store reads pages, not rows")) })
    }

    /// These messages left `mailbox`: archived, deleted or moved. Said
    /// before the list re-reads, so a count the store holds for the folder
    /// can be kept by subtracting what left rather than paid again in front
    /// of the first row (#1607). Synchronous on purpose: it records a fact
    /// for the next read to act on, and a store with nothing to keep does
    /// nothing, which is this default.
    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let _ = (mailbox, messages);
    }

    /// An account's folders, with their counts as of now.
    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>>;

    /// What the sidebar draws beside Drafts and the Outbox.
    ///
    /// Separate from [`mailboxes`](Self::mailboxes) because the Outbox is not
    /// one: it has no row in `mailboxes` to carry a count, and the Drafts badge
    /// needs a number the cached column deliberately does not hold.
    fn draft_counts(&self, account: AccountId)
    -> Read<'_, postio_storage::repository::DraftCounts>;
}

mod local;
pub use local::LocalStore;
/// How many threaded-folder counts this process has issued. For tests — see
/// the counter's own documentation in `sqlite`.
#[doc(hidden)]
pub use local::{folders_counted, last_thread_skip, unified_counted};
