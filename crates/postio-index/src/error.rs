pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A type of its own so callers can tell "search is broken" from "the store is".
    #[error("{0}")]
    Storage(#[from] postio_storage::Error),

    /// From queries that prepare their own statement because their SQL is built at runtime from a
    /// placeholder list.
    #[error("engine: {0}")]
    Engine(#[from] turso::Error),
}
