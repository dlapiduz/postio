//! Messages written out as `.eml` files, for dragging out of a frontend.
//!
//! Moved from the classic app's export (`docs/archive/specs/005-tui-frontend` T018): an
//! `.eml` file *is* the raw RFC 5322 source, which the sync engine has
//! already put in the blob store as `messages.raw_blob_id`, so an export is
//! a copy, not a serialisation. What each file is called is the frontend's
//! choice, as a saved part's path is; which bytes go in it is the store's
//! owner's.

use std::path::PathBuf;

use postio_model::MessageId;
use postio_runtime::Engine;
use postio_storage::{BlobStore, Store};

/// Write each message's raw source to its path, in the order asked, and
/// answer the paths written.
///
/// # It may reach the network, and only because the user asked
///
/// A message whose raw source has not been backfilled yet has nothing to
/// export, so this asks the engine for it and waits -- the same path, and the
/// same justification, as saving an attachment that was never downloaded.
/// The user dragged these messages by name; fetching them is the thing they
/// asked for. With no engine, that message is an error rather than an empty
/// file, and the export stops there.
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
