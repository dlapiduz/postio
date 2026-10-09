//! The attachment indexer: text out of attachments already on this
//! machine, into the index's attachments half (spec 010 D10, T125).
//!
//! The same shape as the body indexer beside it ([`crate::spawn_body_indexer`]):
//! a catch-up pass at start, then a batched pass a moment after each burst
//! of `BodyLoaded` events, indexing the messages the burst named. A payload
//! fetch announces itself with `BodyLoaded` too, which is "the event that
//! says a blob was stored".
//!
//! # It never fetches
//!
//! The queue is [`attachments_missing_text`](postio_index::index::attachments_missing_text):
//! attachments whose blob is already on this machine. An attachment that was
//! never downloaded is not in it, and nothing here holds a backend or a
//! request queue that could ask for its bytes — the functions take a store
//! and a blob store, and that is all they can reach (FR-050). It is indexed
//! when something the person asked for brings it down.
//!
//! # What it logs
//!
//! Attachment ids, outcomes, unit counts and durations: never a file name,
//! never a word of what was read (FR-051).

use std::time::{Duration, Instant};

use postio_extract::{Extracted, Limits, Outcome};
use postio_index::index::MissingAttachment;
use postio_storage::{BlobStore, Store};

/// How long the indexer waits after a burst of arrivals before it runs,
/// so a backfill's burst becomes one batched write.
const INDEX_ATTACHMENT_DEBOUNCE: Duration = Duration::from_millis(500);

/// How many attachments one batch extracts before it writes. Small: one
/// attachment can be seconds of extraction and a megabyte of text, and
/// the batch holds both in memory until its write.
const INDEX_ATTACHMENT_BATCH: u32 = 8;

/// The pause between batches, so the disk keeps answering searches while
/// a large archive catches up (#500's reason, for bodies).
const INDEX_ATTACHMENT_BREATHER: Duration = Duration::from_millis(25);

/// Start the attachment indexer on `runtime`: a catch-up pass now, then a
/// pass after each burst of `BodyLoaded` on `events`. Ends when the event
/// hub does; with `events` `None` only the catch-up runs.
///
/// Both composition roots start one beside the body indexer: the host's
/// idle passes and the macOS boundary.
pub fn spawn_attachment_indexer(
    database: Store,
    blobs: BlobStore,
    events: Option<postio_core::bridge::EventStream>,
    runtime: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        if let Err(error) = index_local_attachments(&database, &blobs).await {
            tracing::warn!(%error, "the attachment indexer's pass failed: {error}");
        }
        let Some(events) = events else {
            return;
        };
        loop {
            let Some(event) = events.next().await else {
                return;
            };
            let mut pending = Vec::new();
            note_arrival(&mut pending, &event);
            let quiet = tokio::time::sleep(INDEX_ATTACHMENT_DEBOUNCE);
            tokio::pin!(quiet);
            loop {
                tokio::select! {
                    () = &mut quiet => break,
                    next = events.next() => match next {
                        None => return,
                        Some(event) => note_arrival(&mut pending, &event),
                    },
                }
            }
            if pending.is_empty() {
                continue;
            }
            pending.sort_unstable();
            pending.dedup();
            if let Err(error) = index_named_attachments(&database, &blobs, &pending).await {
                tracing::warn!(%error, "indexing the attachments a burst named failed: {error}");
            }
        }
    })
}

/// The message a `BodyLoaded` names; every other event is ignored.
fn note_arrival(pending: &mut Vec<postio_model::MessageId>, event: &postio_core::Event) {
    if let postio_core::Event::BodyLoaded { message, .. } = event {
        pending.push(*message);
    }
}

/// Extract every downloaded attachment with no text from the current
/// extractor, in batches. Answers how many it recorded.
///
/// Safe on every start: on a caught-up store it is one query that finds
/// nothing. A part that fails is recorded as failed and not tried again,
/// so the pass always ends.
pub async fn index_local_attachments(
    database: &Store,
    blobs: &BlobStore,
) -> Result<usize, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut indexed = 0usize;
    let mut last: Vec<postio_model::AttachmentId> = Vec::new();
    loop {
        let connection = database.connect_background().await?;
        let found =
            postio_index::index::attachments_missing_text(&connection, INDEX_ATTACHMENT_BATCH)
                .await?;
        drop(connection);
        if found.is_empty() {
            break;
        }
        // The queue's contract is that indexing a part removes it. If a
        // batch comes back unchanged that contract is broken, and going
        // round again can only spin (#500).
        let ids: Vec<_> = found.iter().map(|missing| missing.attachment).collect();
        if ids == last {
            tracing::warn!(
                batch = ids.len(),
                "an attachment batch made no progress; stopping"
            );
            break;
        }
        let taken = found.len();
        indexed += index_batch(database, blobs, found).await?;
        last = ids;
        if taken < INDEX_ATTACHMENT_BATCH as usize {
            break;
        }
        tokio::time::sleep(INDEX_ATTACHMENT_BREATHER).await;
    }
    if indexed > 0 {
        tracing::info!(
            indexed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "indexed attachments"
        );
    }
    Ok(indexed)
}

/// Extract the downloaded, unextracted attachments of exactly these
/// messages: the event path, which knows what arrived and does not ask the
/// whole mailbox.
pub async fn index_named_attachments(
    database: &Store,
    blobs: &BlobStore,
    messages: &[postio_model::MessageId],
) -> Result<usize, Box<dyn std::error::Error>> {
    if messages.is_empty() {
        return Ok(0);
    }
    let started = Instant::now();
    let mut indexed = 0usize;
    for chunk in messages.chunks(256) {
        let connection = database.connect_background().await?;
        let found = postio_index::index::attachments_missing_text_of(&connection, chunk).await?;
        drop(connection);
        for batch in found.chunks(INDEX_ATTACHMENT_BATCH as usize) {
            indexed += index_batch(database, blobs, batch.to_vec()).await?;
            tokio::time::sleep(INDEX_ATTACHMENT_BREATHER).await;
        }
    }
    if indexed > 0 {
        tracing::info!(
            indexed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "indexed attachments"
        );
    }
    Ok(indexed)
}

/// Read and extract each attachment off the runtime, then write them all
/// under one background write. Answers how many were recorded.
async fn index_batch(
    database: &Store,
    blobs: &BlobStore,
    batch: Vec<MissingAttachment>,
) -> Result<usize, Box<dyn std::error::Error>> {
    // Extract first, outside any transaction: a PDF can take its whole time
    // limit, and the write lock must not be held through it.
    let mut extracted = Vec::with_capacity(batch.len());
    for missing in batch {
        let attachment = missing.attachment;
        let blobs = blobs.clone();
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || extract_one(&blobs, &missing))
            .await
            .unwrap_or(Extracted {
                units: Vec::new(),
                outcome: Outcome::Failed,
            });
        tracing::debug!(
            attachment = attachment.get(),
            outcome = result.outcome.as_str(),
            units = result.units.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "extracted an attachment"
        );
        extracted.push((attachment, result));
    }

    let connection = database.connect_background().await?;
    let _permit = connection
        .write_gate()
        .acquire(postio_storage::WritePriority::Background)
        .await;
    connection.execute_batch("BEGIN IMMEDIATE").await?;
    let mut indexed = 0usize;
    for (attachment, result) in &extracted {
        match postio_index::index::index_attachment_text(&connection, *attachment, result).await {
            Ok(true) => indexed += 1,
            // Gone between the queue and here: expunged, or replaced by a
            // refetch whose own row the next pass will find.
            Ok(false) => {}
            Err(error) => {
                connection.execute_batch("ROLLBACK").await?;
                return Err(error.into());
            }
        }
    }
    connection.execute_batch("COMMIT").await?;
    Ok(indexed)
}

/// One attachment's bytes, read and extracted. A blob the store cannot
/// produce is a failure to record, like a file that cannot be parsed: either
/// way there is nothing to read, and the part must leave the queue.
///
/// A blob larger than the input limit is not read at all: the extractor
/// would refuse it, and reading it first would hold all of it in memory.
fn extract_one(blobs: &BlobStore, missing: &MissingAttachment) -> Extracted {
    let limits = Limits::default();
    if blobs
        .len_of(&missing.blob)
        .is_ok_and(|length| length > limits.max_input)
    {
        return Extracted {
            units: Vec::new(),
            outcome: Outcome::Skipped(postio_extract::Skip::TooLarge),
        };
    }
    match blobs.get(&missing.blob) {
        Ok(bytes) => {
            postio_extract::extract(&bytes, &missing.mime_type, missing.name.as_deref(), &limits)
        }
        Err(_) => Extracted {
            units: Vec::new(),
            outcome: Outcome::Failed,
        },
    }
}
