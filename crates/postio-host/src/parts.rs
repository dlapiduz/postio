//! One part of a message, fetched if it has to be, for a person who asked.

use postio_core::bridge::EventSink;
use postio_model::ids::{AttachmentId, BlobId, MessageId};
use postio_runtime::Engine;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, Store};

pub use postio_session::reading::{BODY_WAIT, PartSource, locate_part, part_bytes, wait_for_part};

/// Replaces rather than appends: an appended save would corrupt whatever was there.
pub async fn save_part(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: &EventSink,
    message: MessageId,
    attachment: AttachmentId,
    to: &std::path::Path,
) -> Result<(), String> {
    let bytes = part_bytes(database, blobs, engine, Some(events), message, attachment).await?;
    std::fs::write(to, bytes).map_err(|error| error.to_string())
}

/// A count rather than which ones: one complaint per failure would be worse than the save. A
/// failure does not abandon the batch.
pub async fn save_parts(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    events: &EventSink,
    message: MessageId,
    parts: &[(AttachmentId, std::path::PathBuf)],
) -> usize {
    let mut failed = 0;
    for (attachment, to) in parts {
        if save_part(
            database,
            blobs,
            engine.clone(),
            events,
            message,
            *attachment,
            to,
        )
        .await
        .is_err()
        {
            failed += 1;
        }
    }
    failed
}

/// Polls rather than listens: the event stream has exactly one reader, the window, and a second
/// consumer would be a second place deciding what an event means.
/// The deadline turns a server that never answers into a sentence.
pub async fn wait_for_body(database: &Store, message: MessageId) -> Result<BlobId, String> {
    let deadline = std::time::Instant::now() + BODY_WAIT;
    loop {
        // A failed read is usually the writer we are waiting for holding the table: look again;
        // only the deadline ends this.
        match raw_blob(database, message).await {
            Ok(Some(raw)) => return Ok(raw),
            Ok(None) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err("That part did not arrive in time — it is still \
                        downloading, so try again in a moment"
                .into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

pub async fn part_path(
    database: &Store,
    message: MessageId,
    attachment: AttachmentId,
) -> Result<Option<String>, String> {
    Ok(read_message(database, message)
        .await?
        .attachments
        .iter()
        .find(|part| part.id == attachment)
        .and_then(|part| part.part_id.clone()))
}

/// The person asked for these bytes by name, which justifies fetching. Every byte, not the text
/// axis: under ADR 0017
/// the background lane stores no raw source, so `request_body` would leave `raw_blob_id` empty and
/// the wait would run out its deadline.
pub async fn raw_source(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
) -> Result<Vec<u8>, String> {
    let raw = match raw_blob(database, message).await? {
        Some(raw) => raw,
        None => {
            let engine =
                engine.ok_or("This account is not syncing, so that message cannot be fetched")?;
            if engine
                .request_whole_message(message)
                .await
                .map_err(|error| error.message().to_string())?
            {
                wait_for_body(database, message).await?
            } else {
                // "Nothing to fetch" may be a fetch that landed between the look above and the
                // queue's answer (#109): one re-read settles it.
                raw_blob(database, message)
                    .await?
                    .ok_or("There is nothing to fetch for that message")?
            }
        }
    };
    blobs.get(&raw).map_err(|error| error.to_string())
}

pub async fn raw_blob(database: &Store, message: MessageId) -> Result<Option<BlobId>, String> {
    Ok(read_message(database, message).await?.raw_blob_id)
}

/// Never fetched: `None` when the bytes are not here. Written only under `dir`, in a folder
/// readable by this user alone,
/// using only the name's last component so a hostile name cannot climb out.
pub async fn local_copy(
    database: &Store,
    blobs: &BlobStore,
    attachment: AttachmentId,
    dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let found = async {
        let connection = database.read().await.map_err(|error| error.to_string())?;
        postio_storage::sql::first(
            &connection,
            "SELECT message_id, filename, blob_id FROM attachments
              WHERE id = ?1 AND message_id IS NOT NULL",
            [attachment.get()],
            |row| {
                use postio_storage::sql::RowExt as _;
                Ok((
                    MessageId::new(row.col::<i64>(0)?),
                    row.col::<Option<String>>(1)?,
                    row.col::<Option<String>>(2)?,
                ))
            },
        )
        .await
        .map_err(|error| error.to_string())
    }
    .await;
    let (message, name, blob) = match found {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, attachment = attachment.get(), "an attachment to copy could not be read");
            return None;
        }
    };
    let bytes = match blob {
        Some(blob) => blobs.get(&BlobId::new(blob)).ok(),
        // No engine is handed over, so a part not here stays not here.
        None => part_bytes(database, blobs, None, None, message, attachment)
            .await
            .ok(),
    };
    let Some(bytes) = bytes else {
        tracing::debug!(
            attachment = attachment.get(),
            "an attachment's bytes are not on this machine"
        );
        return None;
    };
    let name = name
        .as_deref()
        .and_then(|name| std::path::Path::new(name).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty() && name != "." && name != "..")
        .unwrap_or_else(|| "attachment".to_owned());
    let folder = dir.join(attachment.get().to_string());
    let written = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)
        .and_then(|()| std::fs::write(folder.join(&name), bytes));
    match written {
        Ok(()) => Some(folder.join(name)),
        Err(error) => {
            tracing::warn!(%error, attachment = attachment.get(), "an attachment could not be copied");
            None
        }
    }
}

/// UIDs and mailbox paths are scoped to an account, so another account's engine would fetch
/// unrelated bytes under the
/// same identity or leave the request waiting forever. One indexed read of the account key.
pub async fn account_of(database: &Store, message: MessageId) -> Option<postio_model::AccountId> {
    let connection = database.read().await.ok()?;
    postio_storage::sql::first(
        &connection,
        "SELECT account_id FROM messages WHERE id = ?1",
        [message.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0).map(postio_model::AccountId::new),
    )
    .await
    .ok()?
}

pub async fn read_message(
    database: &Store,
    message: MessageId,
) -> Result<postio_model::Message, String> {
    let connection = database.read().await.map_err(|error| error.to_string())?;
    MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".into())
}

/// Only the file name is used, so a hostile name cannot write anywhere else. Creates the directory.
pub fn private_copy_path(
    base: &std::path::Path,
    message: MessageId,
    attachment: AttachmentId,
    filename: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let name = filename
        .and_then(|name| {
            std::path::Path::new(name)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty() && name != "." && name != "..")
        .unwrap_or_else(|| "part".to_owned());
    let dir =
        base.join("postio")
            .join("parts")
            .join(format!("{}-{}", message.get(), attachment.get()));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|error| error.to_string())?;
    Ok(dir.join(name))
}
