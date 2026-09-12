//! Postio's local store: the schema, the repositories over it, and the
//! content-addressed blob store.
//!
//! # What lives where
//!
//! The database holds **metadata and message text** — everything the message
//! list, threading, search and the sync engine need to answer a question
//! without touching the network. Raw RFC 5322 bytes and attachment payloads
//! live in a content-addressed blob directory; the database stores the blob
//! key and the metadata beside it.
//!
//! Message *bodies* are in the database, as `TEXT`. They were zstd blobs in a
//! column until the engine changed: the full-text index is an index on the
//! body column now rather than a virtual table beside it, and an index cannot
//! tokenise compressed bytes (`specs/004-turso-store`).
//!
//! The types being persisted come from [`postio_model`], which knows nothing
//! about SQL. This crate is the only place that maps between the two.
//!
//! # Opening a store
//!
//! [`Store::open`] is the one call an application needs: it creates the file
//! and its parent directory, opens it encrypted under the key, and applies the
//! schema if the file is new.
//!
//! ```no_run
//! # async fn example() -> Result<(), postio_storage::Error> {
//! # use postio_storage::key::{Purpose, StoreKey};
//! # let key = StoreKey::generate().derive(Purpose::Database);
//! let store = postio_storage::Store::open("postio.db", &key).await?;
//! let connection = store.connect().await?;
//! # let _ = connection;
//! # Ok(())
//! # }
//! ```
//!
//! # There are no migrations
//!
//! [`schema::HEAD`] is the whole schema and it is applied once, to a file that
//! is new. A store written by the old engine cannot be read by this one at
//! all, so there is nothing for a migration to carry forward: such a store is
//! rebuilt by resyncing. [`schema`] says why that is a licence rather than a
//! policy.
//!
//! See `test_support` (behind the `test-support` feature) for throwaway stores
//! in tests.

pub mod blob;
pub mod error;
pub mod key;
mod perm;
pub(crate) mod sql;
pub mod repository;
pub mod schema;
pub mod store;
#[cfg(feature = "test-support")]
pub mod seed;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use blob::{BlobStore, BlobWriter, EvictionReport};
pub use error::{Error, Result};
pub use store::{Connection, Store, WritePriority};
