//! Messages written out as `.eml` files: a copy of the raw source in the blob store, not a
//! serialisation.
//! The frontend chooses each file's name.

use std::path::PathBuf;

use postio_model::MessageId;
use postio_runtime::Engine;
use postio_storage::{BlobStore, Store};

/// Writes each message's raw source to its path, in order; answers the paths written.
/// May reach the network because the user asked: a message whose raw source is not backfilled is
/// requested from the
/// engine and awaited. With no engine that message is an error, not an empty file, and the export
/// stops.
pub async fn write_messages(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    messages: &[(MessageId, PathBuf)],
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for (message, path) in messages {
        let bytes = crate::parts::raw_source(database, blobs, engine.clone(), *message).await?;
        std::fs::write(path, &bytes).map_err(|error| error.to_string())?;
        written.push(path.clone());
    }
    Ok(written)
}
