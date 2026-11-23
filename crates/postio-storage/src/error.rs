pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("engine: {0}")]
    Engine(#[from] turso::Error),

    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Written by a newer Postio or edited by hand; the schema's CHECK constraints keep the
    /// vocabulary closed.
    #[error("column `{column}` holds `{value}`, which this build does not recognise")]
    UnknownEnum { column: &'static str, value: String },

    #[error("this {entity} has not been persisted yet; create it instead of updating it")]
    NotPersisted { entity: &'static str },

    /// `Flagged`, `Snoozed` and `Outbox` are views with no path or sync state. The schema's `CHECK`
    /// refuses them too, but without naming the role.
    #[error(
        "`{role}` is a view over messages filed elsewhere, not a folder; it cannot be stored as a mailbox"
    )]
    RoleIsAView { role: &'static str },

    /// Used by the cross-account move saga (#188), whose phase walk is forward-only: a backward or
    /// skipping walk loses mail.
    #[error("{what}: {reason}")]
    ForbiddenTransition { what: &'static str, reason: String },

    /// An address has one owner; taking it from a visible person is the user's decision, so the
    /// caller names the owner and offers to move it.
    #[error("address {address} already belongs to contact {owner}")]
    AddressOwned { address: i64, owner: i64 },

    #[error("no {entity} with id {id}")]
    NotFound { entity: &'static str, id: i64 },

    /// Written by a newer Postio, or edited by hand.
    #[error("`operation_queue.{column}` does not hold a readable operation: {source}")]
    CorruptPayload {
        column: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// Columns are read through `sql::RowExt`, which treats NULL as `None`; this is its error for a
    /// type mismatch. Always a bug here or a foreign store.
    #[error("{column}: {reason}")]
    ColumnType { column: String, reason: String },

    /// The caller should not have offered undo; see
    /// [`Operation::inverse`](postio_model::Operation::inverse).
    #[error("a `{op_type}` operation cannot be undone")]
    NotUndoable { op_type: &'static str },

    /// Nothing is looked up: an id from a corrupt row must not name a path of its choosing.
    #[error("`{id}` is not a valid blob key")]
    InvalidBlobId { id: String },

    /// Not folded into [`Error::Io`]: the bytes were readable and making sense of them failed;
    /// never guess under a digest that promises exact content.
    #[error("a stored blob could not be decoded: {reason}")]
    UnreadableBlob { reason: String },

    /// **The database is not damaged**: the key is wrong or replaced. Its own variant because the
    /// engine reports a wrong key in the vocabulary of corruption, which would reach a screen
    /// (#404).
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
    /// The engine enforces one open Postio per store with a file lock. Its own variant so the
    /// message says what to do; the lock records no owner.
    #[error("Postio is already open in another window. Close it to open Postio here.")]
    InUse,
    /// The case [`Error::WrongStoreKey`] cannot catch: an older build's store decrypts fine and
    /// would fail later as `no such column`.
    /// No migration step starts from this stamp, so the remedy is starting over
    /// (`postio_session::start_over`).
    #[error(
        "the local store was written by a build of Postio this one cannot carry \
         forward: its schema is stamped {} and no migration leads from there \
         to this build's {}. Starting a fresh store syncs the mail again from \
         the server; the file is intact and untouched until then",
        stamp(*.found),
        stamp(*.expected)
    )]
    SchemaFromAnotherBuild { found: i64, expected: i64 },
    /// A column of the wrong type; deliberately loud, since an empty reading pane looks like a
    /// message with nothing in it (#70).
    #[error("a stored message body could not be decoded: {reason}")]
    UnreadableBody { reason: String },

    #[error("no blob stored under `{id}`")]
    BlobNotFound { id: String },
    /// The queue and drafts are the only data not copied from a server, so encryption waits for the
    /// drain (ADR 0014 Q4).
    #[error(
        "the store has {pending} operation(s) that have not reached the server yet; \
         they must be sent or discarded before the store can be encrypted"
    )]
    QueueNotDrained { pending: usize },
}

impl Error {
    /// A late writer is not wrong: it lost the lock past `busy_timeout`, or its snapshot went
    /// stale; a caller that can should retry (#1594).
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Error::Engine(turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
        )
    }
}

/// Lives here because both types belong to other crates (`StoreError` is in `postio-model`, ADR
/// 0041).
impl From<Error> for postio_model::listing::StoreError {
    fn from(error: Error) -> Self {
        postio_model::listing::StoreError::new(error.to_string())
    }
}

fn stamp(stamp: i64) -> String {
    format!("{:08x}", stamp as i32 as u32)
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
