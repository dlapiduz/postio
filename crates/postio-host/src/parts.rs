//! One part of a message, fetched if it has to be, for a person who asked.
//!
//! Moved from the classic app (`docs/archive/specs/005-tui-frontend` T046): what a part's bytes
//! are, and how they are waited for, is the store's owner's business, and
//! every frontend saves and opens parts through it. The prose below is the
//! desktop app's, and still true.

use postio_model::ids::{AttachmentId, BlobId, MessageId};
use postio_runtime::Engine;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, Store};

/// How long a save waits for a body it had to ask for.
///
/// Long enough for a slow server on a bad link, short enough that a save that
/// is never going to work says so while the user is still looking at it.
pub const BODY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Where one part's bytes are, when they are on this machine at all.
pub enum PartSource {
    /// The part's own blob — what ADR 0017's payload axis writes into
    /// `attachments.blob_id` when somebody opens an attachment.
    Payload(BlobId),
    /// The whole raw message, from which the part is cut.
    ///
    /// Two rows still land here: one fetched before the payload axis existed,
    /// and one whose `BODYSTRUCTURE` was never recorded, so no section could
    /// be named and every byte was the only answer.
    Raw(BlobId),
}

/// One part's bytes, fetched first if they are not on this machine yet.
///
/// # Why this is the seam rather than the save handler
///
/// `PartsPanel::save_part` runs the portal dialog itself and hands back the
/// file the user chose, so the only part worth testing is what happens next —
/// and that half is nothing to do with GTK. Keeping it here, taking store
/// handles and returning bytes, makes "saves a part that was never
/// downloaded, fetching it first" an ordinary async test over a mock server
/// instead of something that needs a display and a file chooser.
///
/// # Where a received part's bytes are
///
/// In `Attachment::blob_id`, once somebody has opened it. That column was
/// filled only on the way *out* for the whole life of this project — a
/// composer attaching a file — and the receive path stored the whole raw
/// message instead, so a part had to be cut back out of it with `mime::parse`
/// on every open. ADR 0017 ended that: the text axis stores no raw source at
/// all, and the payload axis fetches `BODY.PEEK[<part_id>]` on demand.
///
/// So the fetch to wait for is the *part's*, and asking twice costs nothing:
/// the second open reads the blob and never reaches the network.
///
/// Returns `Err` rather than an empty file when the bytes cannot be had. A
/// zero-byte attachment on disk looks like a saved file and is not one.
pub async fn part_bytes(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    attachment: AttachmentId,
) -> Result<Vec<u8>, String> {
    // Resolved once, before anything is fetched, and deliberately.
    //
    // A whole-message fetch REPLACES the message's attachment rows -- the
    // parser re-reads the structure and `MessageRepository::update` writes the
    // new set -- so the `AttachmentId` the panel is holding does not survive
    // it. The MIME path does: `2` is `2` in every parse of the same bytes. So
    // the id is turned into a path here, while it still means something, and
    // the path is what is used on the far side.
    let part_id = part_path(database, message, attachment)
        .await?
        .ok_or("That part has no place in the message to read it from")?;

    let source = match locate_part(database, message, &part_id).await? {
        Some(source) => source,
        // Never downloaded. This is the one place in the reading pane allowed
        // to reach the network, and only because the user asked for these
        // bytes by name.
        None => {
            let engine =
                engine.ok_or("This account is not syncing, so that part cannot be fetched")?;
            // `request_payloads` puts the section at the front of the backfill
            // and returns as soon as it is queued -- `true` means "there was
            // something to fetch", not "here it is". The bytes land when the
            // engine's own loop claims the job, so the wait is ours.
            let queued = engine
                .request_payloads(message, vec![part_id.clone()])
                .await
                .map_err(|error| error.message().to_string())?;
            tracing::debug!(
                message = message.get(),
                queued,
                "asked the engine for a part"
            );
            if queued {
                wait_for_part(database, message, &part_id).await?
            } else {
                // "Nothing to fetch" has two readings, and the queue cannot
                // tell them apart: there is truly nothing (the message is
                // gone, or AttachmentPolicy::Never), or the background lane
                // fetched this very message between the look above and the
                // queue's answer -- ADR 0016 backfills every mailbox, so
                // both lanes chase the same messages, and the open that
                // races the backfill is an ordinary open, not a corner
                // (#109, four observed failures; a5735a3 is the same race
                // in the runtime's own test). One re-read settles it: a
                // committed write that made the answer `false` is visible
                // to this read, so no wait is needed -- absent here means
                // absent, and the sentence below is then the truth.
                locate_part(database, message, &part_id)
                    .await?
                    .ok_or("There is nothing to fetch for that part")?
            }
        }
    };

    match source {
        PartSource::Payload(blob) => blobs.get(&blob).map_err(|error| error.to_string()),
        PartSource::Raw(blob) => {
            let bytes = blobs.get(&blob).map_err(|error| error.to_string())?;
            postio_model::mime::parse(&bytes)
                .parts
                .into_iter()
                .find(|part| part.attachment.part_id.as_deref() == Some(part_id.as_str()))
                .map(|part| part.content)
                .ok_or_else(|| "That part is not in the message the server sent".into())
        }
    }
}

/// Write one part's bytes to `to`, fetching them first if they were never
/// downloaded -- see [`part_bytes`]. Replaces rather than appends: an
/// appended save would corrupt whatever was there.
pub async fn save_part(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    attachment: AttachmentId,
    to: &std::path::Path,
) -> Result<(), String> {
    let bytes = part_bytes(database, blobs, engine, message, attachment).await?;
    std::fs::write(to, bytes).map_err(|error| error.to_string())
}

/// [`save_part`] for each of `parts`, and how many could not be saved.
///
/// A count rather than which ones: `S` can easily name a dozen parts, and one
/// complaint per failure would be worse than the save. A failure does not
/// abandon the batch -- the parts after it are still saved.
pub async fn save_parts(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    parts: &[(AttachmentId, std::path::PathBuf)],
) -> usize {
    let mut failed = 0;
    for (attachment, to) in parts {
        if save_part(database, blobs, engine.clone(), message, *attachment, to)
            .await
            .is_err()
        {
            failed += 1;
        }
    }
    failed
}

/// Wait for a queued body to land, or give up saying so.
///
/// Polling rather than listening: the engine announces arrivals on the event
/// stream, but that stream has exactly one reader — the window — and a second
/// consumer here would be a second place deciding what an event means. A save
/// the user is waiting on can afford to look.
///
/// The deadline is what turns a server that never answers into a sentence
/// rather than a spinner that never stops.
pub async fn wait_for_body(database: &Store, message: MessageId) -> Result<BlobId, String> {
    let deadline = std::time::Instant::now() + BODY_WAIT;
    loop {
        // A read that fails here is usually the writer we are waiting for
        // holding the table, so contention is a reason to look again rather
        // than to give up. Only the deadline ends this.
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

/// Wait for a queued part to land, or give up saying so.
///
/// [`wait_for_body`]'s sibling, and the same polling for the same reason. It
/// watches for either shape the bytes can arrive in: the part's own blob,
/// which is what a payload fetch writes, and the raw message, which is what
/// the whole-message fallback writes for a row whose section could not be
/// named.
pub async fn wait_for_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
) -> Result<PartSource, String> {
    let started = std::time::Instant::now();
    let deadline = started + BODY_WAIT;
    loop {
        // A read that fails here is usually the writer we are waiting for
        // holding the table, so contention is a reason to look again rather
        // than to give up. Only the deadline ends this.
        match locate_part(database, message, part_id).await {
            Ok(Some(source)) => {
                tracing::debug!(
                    message = message.get(),
                    after_ms = started.elapsed().as_millis() as u64,
                    "a part asked for arrived"
                );
                return Ok(source);
            }
            Ok(None) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            tracing::debug!(message = message.get(), "a part asked for never arrived");
            return Err("That part did not arrive in time — it is still \
                        downloading, so try again in a moment"
                .into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The MIME path of one attachment row, while the row id still means
/// something.
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

/// Whether `part_id`'s bytes are on this machine, and in which shape.
///
/// The part's own blob first: it is the exact bytes, and reading it costs a
/// file open where the raw message costs a parse of the whole thing.
pub async fn locate_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
) -> Result<Option<PartSource>, String> {
    let row = read_message(database, message).await?;
    if let Some(blob) = row
        .attachments
        .iter()
        .find(|part| part.part_id.as_deref() == Some(part_id))
        .and_then(|part| part.blob_id.clone())
    {
        return Ok(Some(PartSource::Payload(blob)));
    }
    Ok(row.raw_blob_id.map(PartSource::Raw))
}

/// `message`'s raw RFC 822 source, every byte as the server sent it: from the
/// blob store when it is here, or fetched now when it is not.
///
/// Fetching is justified the way saving a never-downloaded part is: the
/// person asked for these bytes by name (`view_source`, an export). Every
/// byte, not the text axis: under ADR 0017 the background lane stores no raw
/// source, so `request_body` would fetch the words, leave `raw_blob_id`
/// empty, and the wait would run out its deadline for bytes nothing was
/// fetching.
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
                // "Nothing to fetch" may be a fetch that landed between the
                // look above and the queue's answer (#109's race, as in
                // `part_bytes`): one re-read settles it.
                raw_blob(database, message)
                    .await?
                    .ok_or("There is nothing to fetch for that message")?
            }
        }
    };
    blobs.get(&raw).map_err(|error| error.to_string())
}

/// Just the raw-message blob key. What the wait watches for.
pub async fn raw_blob(database: &Store, message: MessageId) -> Result<Option<BlobId>, String> {
    Ok(read_message(database, message).await?.raw_blob_id)
}

/// A copy of `attachment`'s bytes in `dir`, when they are on this machine
/// (spec 010 FR-050, FR-053): for the system's Quick Look, or a save from
/// the Files tab. Never fetched -- a part whose bytes are not here is
/// `None` -- and written only under `dir`, in a folder of the
/// attachment's own (readable by this user alone), named as the sender
/// named it but only by the name's last component, so a hostile name
/// cannot climb out.
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
        // Cut from a raw message on this machine, never fetched: no engine
        // is handed over, so a part not here stays not here.
        None => part_bytes(database, blobs, None, message, attachment)
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

/// A message's row, or a sentence saying it is gone.
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
