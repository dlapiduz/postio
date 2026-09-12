//! A `rusqlite`-shaped API over Turso.
//!
//! **The point is the shape, not the abstraction.** `postio-storage` is 9,495
//! lines and 265 `rusqlite` call sites, with ~190 more across `postio-runtime`,
//! `-session`, `-sync`, `-index` and `-app`. Porting those by hand to an async
//! API is a rewrite; porting them by
//!
//! ```ignore
//! use postio_turso as rusqlite;
//! ```
//!
//! is a line. So this crate deliberately imitates `rusqlite`'s names and
//! signatures rather than designing anything: `Connection::prepare`,
//! `Statement::query_map`, `Row::get`, `params!`, `OptionalExtension`.
//! Everything it does not implement is something the storage layer does not
//! use, and adding one is a small edit rather than a design decision.
//!
//! # Sync over async, and why it is a thread
//!
//! Turso is async; every caller here is not. The obvious answer —
//! `Handle::block_on` against the ambient runtime — is wrong twice over: it
//! panics inside an async context, and `postio-storage`'s reads run on
//! `spawn_blocking` threads belonging to the very runtime that would be
//! blocked. A current-thread runtime *owned by the connection* has neither
//! problem: nothing else runs on it, and blocking it blocks only this
//! connection, which is what a synchronous database handle means anyway.
//!
//! # What is deliberately different
//!
//! [`Statement::query_map`] materialises. `rusqlite` hands back a lazy
//! iterator borrowing the statement; Turso's rows are an async stream, and
//! bridging laziness across that seam is not worth what it would cost here.
//! Every paged read in `postio-storage` is `LIMIT`ed and small, so this is
//! safe for them — but it is exactly the property `docs/PRODUCT.md` §18 means
//! by "never load a whole mailbox into memory", so an unbounded query through
//! this shim is a bug this crate cannot catch. The places that read without a
//! limit are the backfill passes, and they are the ones to watch.

use std::sync::Arc;

mod error;
mod params;
mod row;
mod statement;

pub use error::{Error, Result};
pub use params::{Params, ToSql};
pub use row::{Row, RowIndex};
pub use statement::{MappedRows, Statement};

/// Rows that may or may not be there — `rusqlite`'s own extension trait, and
/// the storage layer uses it in six places.
pub trait OptionalExtension<T> {
    /// `Ok(None)` where the query returned no rows, rather than an error.
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// How a database is encrypted, or that it is not.
///
/// Turso takes this per database rather than through a pragma, and the cipher
/// names are its own — `aes256gcm`, `aegis256`, and the rest.
#[derive(Debug, Clone)]
pub struct Encryption {
    /// The cipher name Turso knows.
    pub cipher: String,
    /// The key, hex-encoded. 256 bits for the `aes256gcm` this is used with.
    pub hexkey: String,
}

/// One connection to one database, and the runtime that drives it.
///
/// Not `Sync`, like `rusqlite::Connection`: the runtime inside is a
/// current-thread one and driving it from two threads at once is exactly what
/// it cannot do. A pool hands out one per checkout, which is what
/// `postio-storage`'s already does.
pub struct Connection {
    runtime: tokio::runtime::Runtime,
    inner: turso::Connection,
    /// Held so the database outlives the connection taken from it.
    _database: Arc<turso::Database>,
}

impl Connection {
    /// Open `path`, encrypted under `encryption` when there is any.
    ///
    /// The experimental switches are all on: Turso gates encryption,
    /// `WITHOUT ROWID`, the FTS index method and generated columns behind
    /// them, and `postio-storage`'s schema at head uses all four.
    pub fn open(path: &str, encryption: Option<Encryption>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| Error::Backend(format!("no runtime for the store: {source}")))?;

        let database = runtime.block_on(async {
            let mut builder = turso::Builder::new_local(path)
                .experimental_encryption(encryption.is_some())
                .experimental_without_rowid(true)
                .experimental_index_method(true)
                .experimental_generated_columns(true)
                .experimental_strict(true);
            if let Some(encryption) = encryption {
                builder = builder.with_encryption(turso::EncryptionOpts {
                    cipher: encryption.cipher,
                    hexkey: encryption.hexkey,
                });
            }
            builder.build().await
        })?;
        let inner = database.connect()?;
        Ok(Connection {
            runtime,
            inner,
            _database: Arc::new(database),
        })
    }

    /// Open an unencrypted database. For a scratch store in a test.
    pub fn open_plain(path: &str) -> Result<Self> {
        Self::open(path, None)
    }

    /// Run `body` on this connection's runtime.
    pub(crate) fn block_on<F: std::future::Future>(&self, body: F) -> F::Output {
        self.runtime.block_on(body)
    }

    /// Run one or more statements, discarding any rows.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.block_on(async { self.inner.execute_batch(sql).await })?;
        Ok(())
    }

    /// Run one statement, answering how many rows it changed.
    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize> {
        let changed =
            self.block_on(async { self.inner.execute(sql, params.into_values()).await })?;
        Ok(changed as usize)
    }

    /// Prepare `sql` for repeated use.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        let statement = self.block_on(async { self.inner.prepare(sql).await })?;
        Ok(Statement::new(self, statement))
    }

    /// [`Connection::prepare`], against Turso's own statement cache.
    pub fn prepare_cached(&self, sql: &str) -> Result<Statement<'_>> {
        let statement = self.block_on(async { self.inner.prepare_cached(sql).await })?;
        Ok(Statement::new(self, statement))
    }

    /// The one row `sql` returns, mapped by `f`.
    ///
    /// [`Error::QueryReturnedNoRows`] when there is none, which is what
    /// [`OptionalExtension::optional`] turns back into `None`.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T>,
    {
        let mut statement = self.prepare(sql)?;
        statement.query_row(params, f)
    }

    /// The rowid of the last successful insert on this connection.
    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.last_insert_rowid()
    }

    /// Whether the connection is outside an explicit transaction.
    pub fn is_autocommit(&self) -> bool {
        self.inner.is_autocommit().unwrap_or(true)
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}
