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

mod settings;

// Ported back one at a time -- see specs/004-turso-store tasks T016-T026.
// mod accounts;
// mod contact_groups;
// mod contacts;
// mod cross_account;
// mod drafts;
// mod egress;
// mod labels;
// mod mailbox_roles;
// mod mailboxes;
// mod messages;
// mod operations;
// mod sync_state;
// mod threading;
// mod threads;
// mod unsubscribe;

// pub use accounts::{AccountRepository, IdentityRepository, SignatureRepository};
// pub use contact_groups::ContactGroupRepository;
// pub use contacts::ContactRepository;
// pub use cross_account::{
//     CrossAccountMove, CrossAccountMoveRepository, MovePhase, NewCrossAccountMove,
// };
// pub use drafts::{CancelSendOutcome, DraftRepository, ServerCopyLocation};
// pub use egress::EgressLogRepository;
// pub use labels::LabelRepository;
// pub use mailbox_roles::MailboxRoleRepository;
// pub use mailboxes::{DraftCounts, MailboxRepository};
// pub use operations::{OperationQueueRepository, QueuedOperation};
pub use settings::SettingsRepository;
// pub use sync_state::SyncStateRepository;
// pub use threading::{Threaded, ThreadingRepository};
// pub use threads::{
//     DEFAULT_THREAD_PAGE_SIZE, ThreadCursor, ThreadGroup, ThreadListQuery, ThreadListRow,
//     ThreadOrder, ThreadRepository, UnifiedThreadListQuery,
// };
// pub use unsubscribe::UnsubscribeRepository;

// pub use messages::{
//     BackfillCandidate, ColumnFlag, DEFAULT_PAGE_SIZE, FlagSource, ListCursor, ListQuery, ListScope,
//     MessageListRow, MessageRepository, MessageSet, StorageFootprint, StoredBody, UpsertReport,
// };

use chrono::{DateTime, Utc};
use crate::store::Connection;

use crate::error::{Error, Result};

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
