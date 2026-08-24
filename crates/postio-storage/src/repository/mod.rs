//! Repositories: the only place that maps between [`postio_model`] and SQL.
//!
//! # Shape
//!
//! Each repository is a thin borrow of a connection — `AccountRepository::new(&connection)` —
//! so it costs nothing to make one per call and none of them own state. A
//! caller that has checked a connection out of the pool builds whichever
//! repositories it needs and drops them with the connection.
//!
//! # Conventions
//!
//! * **`create` assigns the id.** It takes `&mut` and writes the row id back
//!   into the value, so the caller holds a persisted entity afterwards rather
//!   than having to thread an id around by hand.
//! * **`get` returns `Option`.** A missing row is not an error; a broken row is.
//! * **`delete` returns `bool`** — whether there was anything to delete —
//!   because "already gone" is the expected outcome of a retried operation.
//! * **A write that spans tables runs in one transaction.** An account and its
//!   identities, a mailbox and its sync state: never half of one. Repositories
//!   open that transaction as a [`Scope`], which is a `SAVEPOINT` rather than a
//!   `BEGIN` — so the same call also composes inside a transaction the *caller*
//!   opened. The sync engine depends on that: the messages it fetched and the
//!   sync state describing them have to commit together, and they are written
//!   by two different repositories.
//! * **Timestamps are integer milliseconds, UTC**, and enums are stored as the
//!   `as_str` spelling the model documents, which the schema's `CHECK`
//!   constraints then enforce.

mod accounts;
mod contacts;
mod drafts;
mod mailboxes;
mod messages;
mod operations;
mod sync_state;
mod threading;
mod threads;

pub use accounts::{AccountRepository, IdentityRepository};
pub use contacts::ContactRepository;
pub use drafts::DraftRepository;
pub use mailboxes::MailboxRepository;
pub use operations::{OperationQueueRepository, QueuedOperation};
pub use sync_state::SyncStateRepository;
pub use threading::{Threaded, ThreadingRepository};
pub use threads::{
    DEFAULT_THREAD_PAGE_SIZE, ThreadCursor, ThreadListQuery, ThreadListRow, ThreadOrder,
    ThreadRepository,
};

pub use messages::{
    BackfillCandidate, BodyBlobs, ColumnFlag, DEFAULT_PAGE_SIZE, FlagSource, ListCursor, ListQuery,
    ListScope, MessageListRow, MessageRepository, MessageSet, UpsertReport,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::error::{Error, Result};

/// An atomic write scope: a `SAVEPOINT` that rolls back unless it is committed.
///
/// # Why not `Connection::transaction`
///
/// SQLite has no nested `BEGIN`, so a repository that opens a transaction of
/// its own cannot be called from inside one — and every mutating action in
/// Postio is local-first, which means the interesting writes are exactly the
/// ones that span repositories. A savepoint nests, and behaves like a plain
/// transaction when there is nothing to nest inside, so one spelling covers
/// both.
///
/// Dropping without [`Scope::commit`] rolls back, so an early `?` cannot leave
/// half a write behind.
#[derive(Debug)]
pub(crate) struct Scope<'a> {
    connection: &'a Connection,
    committed: bool,
}

/// One name for every scope. SQLite resolves `RELEASE`/`ROLLBACK TO` against
/// the most recent savepoint of that name, and scopes are nested lexically, so
/// they are always released in the order that resolution expects.
const SAVEPOINT: &str = "postio_scope";

impl<'a> Scope<'a> {
    /// Opens a scope on `connection`.
    pub(crate) fn open(connection: &'a Connection) -> Result<Self> {
        connection.execute_batch(&format!("SAVEPOINT {SAVEPOINT}"))?;
        Ok(Self {
            connection,
            committed: false,
        })
    }

    /// Keeps everything written in this scope.
    ///
    /// When this is the outermost scope the write is durable on return; when it
    /// is nested, it becomes part of the enclosing scope's fate.
    pub(crate) fn commit(mut self) -> Result<()> {
        self.committed = true;
        self.connection
            .execute_batch(&format!("RELEASE {SAVEPOINT}"))?;
        Ok(())
    }
}

impl std::ops::Deref for Scope<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Best effort: the caller is already unwinding an error, and a failure
        // to roll back is reported by the next statement on this connection.
        let _ = self
            .connection
            .execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}"));
    }
}

/// A timestamp as the schema stores it: milliseconds since the Unix epoch, UTC.
pub(crate) fn to_millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

/// The inverse of [`to_millis`].
///
/// A value the database cannot represent as a timestamp is clamped rather than
/// dropped: it came out of a row, so something is there, and refusing to show
/// the message would be worse than showing it with an odd date.
pub(crate) fn from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or(if millis < 0 {
        DateTime::<Utc>::MIN_UTC
    } else {
        DateTime::<Utc>::MAX_UTC
    })
}

/// Fails a read whose row holds a value this build does not understand.
///
/// The schema's `CHECK` constraints keep the enum columns to a known
/// vocabulary, so this only fires for a database written by a newer Postio or
/// edited by hand — in both cases guessing is worse than saying so.
pub(crate) fn unknown_enum(column: &'static str, value: impl Into<String>) -> Error {
    Error::UnknownEnum {
        column,
        value: value.into(),
    }
}

/// The `id` of a value that must already be persisted.
pub(crate) fn require_persisted(id: i64, entity: &'static str) -> Result<i64, Error> {
    if id > 0 {
        Ok(id)
    } else {
        Err(Error::NotPersisted { entity })
    }
}
