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
        "the local store will not open: it belongs to another installation, \
         the keyring entry has been replaced, or it was written by a Postio \
         from before the storage engine changed. The file is intact and \
         untouched either way -- nothing here rewrites a store it cannot \
         read. A store that cannot be opened is rebuilt by syncing again, \
         which costs the mail's download and loses nothing the server still \
         has"
    )]
    WrongStoreKey,
    /// The store opened, but its schema is not the one this build expects.
    ///
    /// Its own variant because it is the case [`Error::WrongStoreKey`] cannot
    /// reach. A store written by an earlier build of *this* engine decrypts
    /// perfectly — same cipher, same key, same file format — so nothing at the
    /// door objects, and the mismatch surfaces later as `no such column` on
    /// whichever statement names something added since, one statement at a
    /// time, indefinitely.
    ///
    /// There are no migrations (`crate::schema::HEAD` says why), so the remedy
    /// is to sync again, and this is the only thing in a position to say so.
    #[error(
        "the local store was written by a different build of Postio: its \
         schema is stamped {found} and this build expects {expected}. There \
         are no migrations -- a store is rebuilt by syncing again, which costs \
         the mail's download and loses nothing the server still has. The file \
         is intact and untouched: nothing here rewrites a store it will not use"
    )]
    SchemaFromAnotherBuild {
        /// The fingerprint the file carries.
        found: i64,
        /// The fingerprint this build's schema hashes to.
        expected: i64,
    },
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

impl Error {
    /// Whether the engine turned a write away because another connection
    /// held the lock past `busy_timeout` (#1594).
    ///
    /// The gate orders Postio's own writers and the timeout covers whatever
    /// it does not; a writer that outlives both is not wrong, it is late,
    /// and a caller that can try again should. Two variants because the
    /// engine names a snapshot that went stale under a reader separately
    /// from a lock it could not take, and both mean "not now".
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Error::Engine(turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
        )
    }
}

#[cfg(test)]
mod busy_tests {
    use super::*;

    #[test]
    fn the_engine_saying_busy_is_busy() {
        assert!(Error::Engine(turso::Error::Busy("database is locked".to_owned())).is_busy());
        assert!(Error::Engine(turso::Error::BusySnapshot("snapshot".to_owned())).is_busy());
    }

    #[test]
    fn any_other_error_is_not() {
        assert!(!Error::Engine(turso::Error::QueryReturnedNoRows).is_busy());
        assert!(!Error::WrongStoreKey.is_busy());
    }
}
