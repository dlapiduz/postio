//! The error type, shaped like `rusqlite`'s where the storage layer matches on
//! it.

/// What this crate returns.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A failure from Turso, or from the shim around it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `query_row` found nothing. Named as `rusqlite` names it, because
    /// `OptionalExtension` and six call sites match on exactly this.
    #[error("query returned no rows")]
    QueryReturnedNoRows,
    /// A column index or name that is not in the result.
    #[error("no such column: {0}")]
    InvalidColumnName(String),
    /// A value that is not the type the caller asked for.
    #[error("cannot read column {index} as {wanted}: it is {found}")]
    InvalidColumnType {
        /// Which column.
        index: usize,
        /// The Rust type asked for.
        wanted: &'static str,
        /// What Turso actually had.
        found: String,
    },
    /// Anything Turso itself refused.
    #[error("{0}")]
    Turso(#[from] turso::Error),
    /// Anything this shim could not do.
    #[error("{0}")]
    Backend(String),
}
