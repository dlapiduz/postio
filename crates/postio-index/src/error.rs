//! Errors the search index and executor can return.

/// The search layer's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Something went wrong building or querying the FTS5 index.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// SQLite reported an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}
