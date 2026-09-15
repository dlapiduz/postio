//! Errors the search index and executor can return.

/// The search layer's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Something went wrong building or querying the search index.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The storage layer reported an error.
    ///
    /// Wraps rather than aliases: this crate writes and reads the same
    /// database, through the same accessors, so a failure here is one of
    /// theirs — but callers distinguish "search is broken" from "the store
    /// is", and a type of its own is what lets them.
    #[error("{0}")]
    Storage(#[from] postio_storage::Error),

    /// The engine reported an error on a statement this crate ran directly.
    ///
    /// A few queries here prepare their own statement rather than going
    /// through `postio_storage::sql` -- the ones whose SQL is built at
    /// runtime from a placeholder list -- and those surface the engine's
    /// error rather than the storage layer's.
    #[error("engine: {0}")]
    Engine(#[from] turso::Error),
}
