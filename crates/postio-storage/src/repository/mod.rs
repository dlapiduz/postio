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
//!   open that transaction as a [`Scope`], which becomes a `SAVEPOINT` when it
//!   is nested — so the same call also composes inside a transaction the
//!   *caller* opened. The sync engine depends on that: the messages it fetched
//!   and the sync state describing them have to commit together, and they are
//!   written by two different repositories. An outermost `Scope` is a
//!   `BEGIN IMMEDIATE` instead, which is not an optimisation — see [`Scope`].
//! * **Timestamps are integer milliseconds, UTC**, and enums are stored as the
//!   `as_str` spelling the model documents, which the schema's `CHECK`
//!   constraints then enforce.

mod accounts;
mod contact_groups;
mod contacts;
mod cross_account;
mod drafts;
mod egress;
mod mailboxes;
mod messages;
mod operations;
mod settings;
mod sync_state;
mod threading;
mod threads;

pub use accounts::{AccountRepository, IdentityRepository, SignatureRepository};
pub use contact_groups::ContactGroupRepository;
pub use contacts::ContactRepository;
pub use cross_account::{
    CrossAccountMove, CrossAccountMoveRepository, MovePhase, NewCrossAccountMove,
};
pub use drafts::{DraftRepository, ServerCopyLocation};
pub use egress::EgressLogRepository;
pub use mailboxes::MailboxRepository;
pub use operations::{OperationQueueRepository, QueuedOperation};
pub use settings::SettingsRepository;
pub use sync_state::SyncStateRepository;
pub use threading::{Threaded, ThreadingRepository};
pub use threads::{
    DEFAULT_THREAD_PAGE_SIZE, ThreadCursor, ThreadGroup, ThreadListQuery, ThreadListRow,
    ThreadOrder, ThreadRepository, UnifiedThreadListQuery,
};

pub use messages::{
    BackfillCandidate, ColumnFlag, DEFAULT_PAGE_SIZE, FlagSource, ListCursor, ListQuery, ListScope,
    MessageListRow, MessageRepository, MessageSet, StorageFootprint, StoredBody, UpsertReport,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::error::{Error, Result};

/// An atomic write scope that rolls back unless it is committed.
///
/// # Why not `Connection::transaction`
///
/// SQLite has no nested `BEGIN`, so a repository that opens a transaction of
/// its own cannot be called from inside one — and every mutating action in
/// Postio is local-first, which means the interesting writes are exactly the
/// ones that span repositories. A savepoint nests, so one type covers both the
/// enclosing write and the repository call inside it.
///
/// Dropping without [`Scope::commit`] rolls back, so an early `?` cannot leave
/// half a write behind.
///
/// # The outermost scope is `BEGIN IMMEDIATE`, and that is load-bearing
///
/// A bare `SAVEPOINT` outside any transaction *starts* one, and the one it
/// starts is deferred — it takes no lock until something asks for one. Every
/// scope in this module then reads before it writes, because that is what a
/// read-modify-write is: `SyncStateRepository::mutate` loads the state before
/// saving it, `upsert_batch` looks a UID up before deciding insert or update.
/// So a deferred scope is holding a *read* lock by the time it writes and has
/// to promote — and SQLite will not let a promotion wait. Blocking a
/// connection that already holds a read lock could deadlock against the writer
/// it would be waiting for, so it returns `SQLITE_BUSY` and deliberately does
/// not invoke the busy handler, which is the exemption
/// `sqlite3_busy_handler`'s own documentation describes. `PRAGMA busy_timeout`
/// (see [`crate::db::PRAGMAS`]) never gets a say and the write fails on the
/// spot.
///
/// Postio always has a second writer — the UI thread, writing local-first on
/// every flag, archive and draft autosave — so this is not theoretical: issue
/// #79 found a sync pass losing its very first batch to a `f` keystroke.
/// Taking the write lock up front, before any read, is what puts these writes
/// back inside the five-second timeout.
///
/// A nested scope stays a plain `SAVEPOINT`: the transaction enclosing it has
/// already resolved the question, and asking again would be a second `BEGIN`.
/// Every scope in this module encloses a write, so there is no read-only
/// caller paying for a write lock it did not need.
#[derive(Debug)]
pub(crate) struct Scope<'a> {
    connection: &'a Connection,
    committed: bool,
    /// Whether this scope began the transaction, and so has to end it.
    outermost: bool,
}

/// One name for every scope. SQLite resolves `RELEASE`/`ROLLBACK TO` against
/// the most recent savepoint of that name, and scopes are nested lexically, so
/// they are always released in the order that resolution expects.
const SAVEPOINT: &str = "postio_scope";

impl<'a> Scope<'a> {
    /// Opens a scope on `connection`.
    ///
    /// Takes the write lock up front when nothing else has already opened a
    /// transaction on this connection — see the type's own docs for why that
    /// is not an optimisation.
    pub(crate) fn open(connection: &'a Connection) -> Result<Self> {
        // `is_autocommit` is false exactly when a transaction is already open,
        // which is what "am I nested" means here — whether the enclosing
        // transaction came from another `Scope` or from a caller's own
        // `BEGIN` makes no difference to what this one has to do.
        let outermost = connection.is_autocommit();
        let sql = if outermost {
            "BEGIN IMMEDIATE".to_owned()
        } else {
            format!("SAVEPOINT {SAVEPOINT}")
        };
        connection.execute_batch(&sql)?;
        Ok(Self {
            connection,
            committed: false,
            outermost,
        })
    }

    /// Keeps everything written in this scope.
    ///
    /// When this is the outermost scope the write is durable on return; when it
    /// is nested, it becomes part of the enclosing scope's fate.
    pub(crate) fn commit(mut self) -> Result<()> {
        self.committed = true;
        let sql = if self.outermost {
            "COMMIT".to_owned()
        } else {
            format!("RELEASE {SAVEPOINT}")
        };
        self.connection.execute_batch(&sql)?;
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
        let sql = if self.outermost {
            "ROLLBACK".to_owned()
        } else {
            format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}")
        };
        // Best effort: the caller is already unwinding an error, and a failure
        // to roll back is reported by the next statement on this connection.
        let _ = self.connection.execute_batch(&sql);
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
