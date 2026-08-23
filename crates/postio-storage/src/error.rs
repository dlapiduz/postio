//! Errors the storage layer can return.

/// The storage layer's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Something went wrong talking to the local database.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// SQLite reported an error that is not specific to migrating.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The filesystem got in the way of opening the database — most often the
    /// data directory could not be created.
    #[error("{path}: {source}")]
    Io {
        /// What was being opened or created.
        path: std::path::PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },

    /// A row holds a value in an enumerated column that this build does not
    /// know. The schema's CHECK constraints keep the vocabulary closed, so this
    /// means a newer Postio wrote the row, or something edited it by hand.
    #[error("column `{column}` holds `{value}`, which this build does not recognise")]
    UnknownEnum {
        /// `table.column` the value came from.
        column: &'static str,
        /// What was there.
        value: String,
    },

    /// A write was asked for against a value that has never been stored.
    #[error("this {entity} has not been persisted yet; create it instead of updating it")]
    NotPersisted {
        /// What kind of thing it was.
        entity: &'static str,
    },

    /// A write named a row that is not there.
    #[error("no {entity} with id {id}")]
    NotFound {
        /// What kind of thing was being written.
        entity: &'static str,
        /// The id that matched nothing.
        id: i64,
    },

    /// A blob key is not a digest of the right shape. Nothing is looked up:
    /// an id from a corrupt row must not be able to name a path of its own
    /// choosing.
    #[error("`{id}` is not a valid blob key")]
    InvalidBlobId {
        /// The offending key.
        id: String,
    },

    /// The blob store has no blob under this key.
    #[error("no blob stored under `{id}`")]
    BlobNotFound {
        /// The key that was looked up.
        id: String,
    },

    /// A migration's SQL failed. That migration was rolled back whole and the
    /// database is still at the last version that committed.
    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        /// The migration that failed.
        version: u32,
        /// Its name.
        name: &'static str,
        /// What SQLite said.
        #[source]
        source: rusqlite::Error,
    },

    /// The database was written by a newer build of Postio. Opening it would
    /// mean guessing at columns this build has never seen, so it refuses.
    #[error(
        "this database is at schema version {found}, but this build of Postio only knows \
         version {known}; it was written by a newer version"
    )]
    SchemaTooNew {
        /// The version recorded in the database.
        found: u32,
        /// The newest version this build ships.
        known: u32,
    },

    /// An already-applied migration no longer matches the one that ran.
    /// Migrations are forward-only: add a new one rather than editing history.
    #[error("migration {version} ({name}) no longer matches the database: {detail}")]
    MigrationChanged {
        /// The version whose record disagrees.
        version: u32,
        /// The name recorded when it was applied.
        name: String,
        /// How it disagrees.
        detail: &'static str,
    },

    /// The migration list is not numbered `1..n` in ascending order. A
    /// programming error, caught before anything is applied.
    #[error(
        "migration list is malformed: expected version {expected} at position {position}, \
         found {found}"
    )]
    MigrationOrder {
        /// Index into the list.
        position: usize,
        /// The version that belongs there.
        expected: u32,
        /// The version that is there.
        found: u32,
    },
}
