//! Postio's local store: the schema, the repositories over it, and the content-addressed blob
//! store.
//! The database holds metadata and message text; raw RFC 5322 bytes and attachment payloads are
//! blobs keyed from it.
//! [`Store::open`] creates, encrypts and migrates the file; [`schema`] says how to add a migration
//! step.

/// SQL for a contact's shown name, for the contact aliased `$a`; `NULL` when they gave no name.
/// The one home of the rule `postio_model::Contact::shown_as` states in Rust; a test holds the two
/// together.
macro_rules! shown_name {
    ($a:literal) => {
        concat!(
            "CASE ",
            $a,
            ".show_as ",
            "WHEN 'first' THEN coalesce(nullif(trim(",
            $a,
            ".given_name), ''), ",
            $a,
            ".name) ",
            "WHEN 'last_first' THEN CASE ",
            "WHEN nullif(trim(",
            $a,
            ".family_name), '') IS NOT NULL ",
            "AND nullif(trim(",
            $a,
            ".given_name), '') IS NOT NULL ",
            "THEN trim(",
            $a,
            ".family_name) || ', ' || trim(",
            $a,
            ".given_name) ",
            "ELSE coalesce(nullif(trim(",
            $a,
            ".family_name), ''), ",
            $a,
            ".name) END ",
            "ELSE ",
            $a,
            ".name END"
        )
    };
}
pub(crate) use shown_name;

pub mod actions;
pub mod blob;
pub mod body_codec;
pub mod error;
pub mod key;
mod perm;
pub mod repository;
pub mod schema;
pub mod searches;
#[cfg(feature = "test-support")]
pub mod seed;
/// Public for `postio-index`, which maintains the search index over the same tables; everything
/// else goes through the repositories.
pub mod sql;
pub mod store;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use blob::{BlobStore, BlobWriter, EvictionReport};
pub use error::{Error, Result};
pub use store::{
    Checkout, Connection, MAX_CONCURRENT_PASSES, Reader, Store, WriteGate, WritePermit,
    WritePriority,
};

/// `BEGIN IMMEDIATE` at the outermost level, a `SAVEPOINT` when nested, so a repository call
/// composes inside a bigger write.
///
/// The error type is the caller's as long as it can carry one of ours, so a sync unit need not
/// `map_err` on every line.
pub async fn transaction<T, E, F, Fut>(
    connection: &Connection,
    work: F,
) -> std::result::Result<T, E>
where
    E: From<Error>,
    F: FnOnce(Connection) -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, E>>,
{
    sql::in_scope(connection, work).await
}
