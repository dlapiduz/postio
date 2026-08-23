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
//!   identities, a mailbox and its sync state: never half of one.
//! * **Timestamps are integer milliseconds, UTC**, and enums are stored as the
//!   `as_str` spelling the model documents, which the schema's `CHECK`
//!   constraints then enforce.

mod accounts;
mod mailboxes;

pub use accounts::{AccountRepository, IdentityRepository};
pub use mailboxes::MailboxRepository;

use chrono::{DateTime, Utc};

use crate::error::Error;

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
