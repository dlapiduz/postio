//! Messages written out as `.eml` files, for dragging out of a frontend.
//!
//! Moved from `postio-app`'s export (`specs/005-tui-frontend` T018): an
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
        let raw = match crate::parts::raw_blob(database, *message).await? {
            Some(raw) => raw,
            None => {
                let engine = engine.clone().ok_or(
                    "This account is not syncing, so that message cannot be fetched to export",
                )?;
                // Every byte, not the text axis: what is being written here
                // is the original RFC 5322 message, and under ADR 0017 the
                // background lane stores no raw source at all. `request_body`
                // would fetch the words, leave `raw_blob_id` empty, and this
                // would wait out its deadline for bytes nothing was fetching.
                if !engine
                    .request_whole_message(*message)
                    .await
                    .map_err(|error| error.message().to_string())?
                {
                    return Err("There is nothing to fetch for that message".into());
                }
                crate::parts::wait_for_body(database, *message).await?
            }
        };

        let bytes = blobs.get(&raw).map_err(|error| error.to_string())?;
        std::fs::write(path, &bytes).map_err(|error| error.to_string())?;
        written.push(path.clone());
    }
    Ok(written)
}
