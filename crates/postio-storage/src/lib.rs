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
//! ```no_run
//! use rusqlite::Connection;
//!
//! # fn main() -> Result<(), postio_storage::Error> {
//! let mut connection = Connection::open("postio.db")?;
//! let report = postio_storage::migrate(&mut connection)?;
//! if !report.is_no_op() {
//!     eprintln!("migrated {} -> {}", report.from, report.to);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`migrate`] is idempotent, so it belongs on every start rather than behind a
//! version check of the caller's own. Connection pragmas and pooling are a
//! separate concern and are not set up here.
//!
//! See [`migrations`] for the rules a schema change has to follow.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod migrations;

pub use error::{Error, Result};
pub use migrations::{Migration, MigrationReport, migrate, schema_version};
