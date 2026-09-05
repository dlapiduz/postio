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

    /// A state transition the domain forbids was asked for.
    ///
    /// First (and so far only) user: the cross-account move saga (#188),
    /// whose phase walk is forward-only precisely because a backward or
    /// skipping walk is how mail gets lost.
    #[error("{what}: {reason}")]
    ForbiddenTransition {
        /// What was being moved.
        what: &'static str,
        /// Why this transition is not allowed.
        reason: String,
    },

    /// A write named a row that is not there.
    #[error("no {entity} with id {id}")]
    NotFound {
        /// What kind of thing was being written.
        entity: &'static str,
        /// The id that matched nothing.
        id: i64,
    },

    /// A queue row's JSON payload could not be read back as an operation.
    /// Either a newer Postio wrote it, or something edited the row by hand.
    #[error("`operation_queue.{column}` does not hold a readable operation: {source}")]
    CorruptPayload {
        /// Which column failed to decode.
        column: &'static str,
        /// What the JSON decoder said.
        #[source]
        source: serde_json::Error,
    },

    /// Undo was asked for on an operation that has no inverse — an expunge, an
    /// append, a send. The caller should not have offered it; see
    /// [`Operation::inverse`](postio_model::Operation::inverse).
    #[error("a `{op_type}` operation cannot be undone")]
    NotUndoable {
        /// The operation's stored `op_type`.
        op_type: &'static str,
    },

    /// A blob key is not a digest of the right shape. Nothing is looked up:
    /// an id from a corrupt row must not be able to name a path of its own
    /// choosing.
    #[error("`{id}` is not a valid blob key")]
    InvalidBlobId {
        /// The offending key.
        id: String,
    },

    /// A blob file exists but this build cannot decode it: it names a
    /// container version or a codec from a newer Postio, or its compressed
    /// payload is damaged.
    ///
    /// Deliberately not folded into [`Error::Io`]: the bytes were readable,
    /// and what failed was making sense of them. Handing back a guess under a
    /// digest that promises exact content is the one thing the blob store must
    /// never do.
    #[error("a stored blob could not be decoded: {reason}")]
    UnreadableBlob {
        /// What about it could not be decoded.
        reason: String,
    },

    /// The database will not decrypt under the key it was given.
    ///
    /// The store belongs to a different installation, or the keyring entry has
    /// been replaced or was written by something else. **The database is not
    /// damaged** — it is intact and locked, and the right key still opens it.
    ///
    /// A variant of its own rather than the `SQLITE_NOTADB` this is made from,
    /// because SQLite's own wording for a wrong key is "file is not a
    /// database", which tells a user their mail is corrupt. That sentence
    /// reaches a screen (#404), and it would be a lie.
    #[error(
        "the local store will not open with this key: it belongs to another \
         installation, or the keyring entry has been replaced. The database \
         itself is intact"
    )]
    WrongStoreKey,

    /// A stored message body could not be compressed or read back.
    ///
    /// The row and this build disagree about what is in the column: a frame
    /// that will not decode, a dictionary the database does not have, or bytes
    /// that are not UTF-8. Deliberately loud rather than an empty body — a
    /// reading pane that renders nothing looks the same as a message that had
    /// nothing in it, and those are opposite facts (#70).
    #[error("a stored message body could not be decoded: {reason}")]
    UnreadableBody {
        /// What about it could not be decoded.
        reason: String,
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

    /// A migration left a row pointing at something that is not there.
    ///
    /// Foreign keys are enforced everywhere else, but they have to be off
    /// while a migration rebuilds a table — a `DROP TABLE` with them on fires
    /// `ON DELETE CASCADE` on the children of the table being replaced. This
    /// is the check that runs afterwards, so a rebuild that dropped a
    /// reference fails the migration instead of leaving a database that looks
    /// right and is not.
    #[error("migrating left {rows} row(s) in `{table}` pointing at a `{parent}` that is not there")]
    MigrationBrokeReferences {
        /// The table holding the dangling rows.
        table: String,
        /// The table they point at.
        parent: String,
        /// How many rows are dangling.
        rows: usize,
    },

    /// A store cannot be encrypted while there are operations the server has
    /// not seen yet.
    ///
    /// The queue and the drafts are the only things in the store that are not
    /// a copy of something on a server, so they are the only things a
    /// migration could actually lose. ADR 0014 Q4's ordering is drain first
    /// for exactly that reason, and this is what stops the migration from
    /// running before the drain has happened.
    #[error(
        "the store has {pending} operation(s) that have not reached the server yet; \
         they must be sent or discarded before the store can be encrypted"
    )]
    QueueNotDrained {
        /// How many rows are still pending or in flight.
        pending: usize,
    },

    /// The encrypted store this migration built did not read back correctly,
    /// so nothing was swapped and the plaintext store is untouched.
    ///
    /// The one failure mode a migration is not allowed to have is losing mail,
    /// which is why the check happens before anything is moved rather than
    /// after.
    #[error("the encrypted store did not verify, so nothing was replaced: {reason}")]
    MigrationDidNotVerify {
        /// What did not read back.
        reason: String,
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
