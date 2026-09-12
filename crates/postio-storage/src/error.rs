//! Errors the storage layer can return.

/// The storage layer's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Something went wrong talking to the local database.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The database engine reported an error.
    #[error("engine: {0}")]
    Engine(#[from] turso::Error),

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

    /// A mailbox was asked to be stored wearing a role that is a *view* over
    /// messages filed elsewhere — `Flagged`, `Snoozed` or `Outbox`.
    ///
    /// A view has a name, a position and a count, and nothing else: no path,
    /// no UIDVALIDITY, no sync state, and no folder on any server. The schema's
    /// `CHECK` refuses these too, but a constraint violation reports
    /// "constraint failed" and does not say *which* role was wrong — and this
    /// is a caller bug worth naming, not a storage accident.
    #[error(
        "`{role}` is a view over messages filed elsewhere, not a folder; it cannot be stored as a mailbox"
    )]
    RoleIsAView {
        /// The offending role, in its stored spelling.
        role: &'static str,
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

    /// A column held something that is not the type the schema declares.
    ///
    /// Under `rusqlite` this was `FromSqlConversionFailure`, and it arrived
    /// with the column's index and SQL type already attached. The engine's own
    /// accessor is sealed and treats NULL as an error rather than as `None`,
    /// so this crate reads columns through `sql::RowExt` and this is what that
    /// returns when the value is not what was asked for.
    ///
    /// Always a bug in this crate or a store written by something else: the
    /// schema is the only writer, and it declares every one of these types.
    #[error("{column}: {reason}")]
    ColumnType {
        /// Which column, by index or by name.
        column: String,
        /// What was expected and what was there.
        reason: String,
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
    /// A variant of its own rather than the engine error it is made from,
    /// because an engine handed the wrong key sees a page that will not
    /// authenticate and reports it in the vocabulary of corruption. That
    /// sentence reaches a screen (#404), and it would be a lie: the file is
    /// intact, and the key is what does not fit it.
    #[error(
        "the local store will not open with this key: it belongs to another \
         installation, or the keyring entry has been replaced. The database \
         itself is intact"
    )]
    WrongStoreKey,
    /// A stored message body could not be read back.
    ///
    /// The row holds something that is not the text it claims to be. Much
    /// narrower than it was: bodies are plain `TEXT` now rather than zstd
    /// frames against a shared dictionary, so the decode that used to fail is
    /// gone and what is left is a column whose type is wrong. Kept because
    /// that is still reachable, and still deliberately loud rather than an
    /// empty body -- a reading pane that renders nothing looks the same as a
    /// message that had nothing in it, and those are opposite facts (#70).
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
}
