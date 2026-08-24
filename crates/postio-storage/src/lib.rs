//! Postio's local store: the SQLite schema, its migration runner, the
//! repositories over it, and the content-addressed blob store.
//!
//! # What lives where
//!
//! SQLite holds **metadata only** — everything the message list, threading,
//! search and the sync engine need to answer a question without touching the
//! network. Message bodies, raw RFC 5322 bytes and attachment payloads live in
//! a content-addressed blob directory; the database stores the blob key and the
//! metadata beside it. That is what keeps the database small enough for the
//! `<16ms` interaction and `<100ms` search budgets in `CLAUDE.md`.
//!
//! The types being persisted come from [`postio_model`], which knows nothing
//! about SQL. This crate is the only place that maps between the two.
//!
//! # Opening a database
//!
//! [`Database::open`] is the one call an application needs: it creates the file
//! and its parent directory, configures every connection with Postio's pragmas
//! (see [`db::PRAGMAS`]), migrates the schema to head, and hands back a pool.
//!
//! ```no_run
//! # fn main() -> Result<(), postio_storage::Error> {
//! let database = postio_storage::Database::open("postio.db")?;
//! let connection = database.connection()?;
//! # let _ = connection;
//! # Ok(())
//! # }
//! ```
//!
//! Migrating is idempotent, so it belongs on every start rather than behind a
//! version check of the caller's own; [`migrate`] is the lower-level entry point
//! for a connection opened some other way.
//!
//! See [`migrations`] for the rules a schema change has to follow, and
//! `test_support` (behind the `test-support` feature) for throwaway databases in
//! tests.

pub mod blob;
pub mod db;
pub mod error;
pub mod migrations;
pub mod repository;
#[cfg(feature = "test-support")]
pub mod seed;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use blob::BlobStore;
pub use db::{Database, Pool, PooledConnection};
pub use error::{Error, Result};
pub use migrations::{Migration, MigrationReport, migrate, schema_version};
