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


/// Which messages the list is showing.
///
/// `postio-model`'s: every reader of the list, from `postio-storage`'s own
/// queries to a frontend's feed, answers this the same way (#670).
pub use postio_model::ListScope;

pub use postio_model::listing::{
    ListPage, ListRows, MessagePage, MessageSummary, PageRequest, StoreError, ThreadPage,
    ThreadSummary,
};

pub use postio_model::listing::{MailStore, Read};

mod local;
pub use local::LocalStore;
/// How many threaded-folder counts this process has issued. For tests — see
/// the counter's own documentation in `sqlite`.
#[doc(hidden)]
pub use local::{folders_counted, last_thread_skip, unified_counted};
