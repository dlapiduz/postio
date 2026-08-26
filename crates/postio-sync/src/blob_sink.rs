//! A [`BodySink`] that writes straight into the blob store.
//!
//! # Why this exists
//!
//! [`VecSink`](postio_imap::backend::VecSink) collects a fetch into a
//! `Vec<u8>`, which is fine for a test and fine for a small part, and wrong for
//! a message. A `Vec` grows by doubling, so a 40 MB attachment peaks well above
//! 40 MB and copies itself a dozen times on the way there — and the background
//! lane's `max_body_bytes` cap does not save anyone from it, because **the
//! interactive lane ignores that cap by design**: the user asked for that
//! message and is watching a spinner. There are 539 messages over 5 MB in the
//! account ADR 0017 measured.
//!
//! So the bytes go socket, chunk, file: this sink holds one chunk as the
//! transport hands it over, the digest is computed as it goes past, and the
//! file on disk is the buffer. That is the rule ADR 0017's second axis states —
//! *no message byte is ever resident in the process except the text being
//! parsed.*
//!
//! # Why it lives here and not in `postio-storage`
//!
//! [`BodySink`] belongs to `postio-imap` and [`BlobStore`] to
//! `postio-storage`; making either depend on the other to join them would
//! invert the layering (`postio-storage` is *below* the protocol, not beside
//! it). `postio-sync` is the crate whose whole job is joining protocol to
//! storage, so the seam belongs here. `postio-storage` supplies the primitive
//! — [`BlobWriter`] — and knows nothing about fetches.
//!
//! # The contract, restated
//!
//! [`BodySink::finish`] is called only when the fetch completed. A sink that
//! never sees it is holding a fragment, and this one publishes nothing:
//! [`BlobSink::finished_blob`] is `None`, the temporary file is left for
//! [`BlobStore::purge_temporary`], and no blob exists under a digest that
//! promises whole bytes. There is deliberately no way to ask for the id of an
//! unfinished write.

use async_trait::async_trait;
use postio_imap::backend::{BackendError, BackendResult, BodySink};
use postio_model::BlobId;
use postio_storage::{BlobStore, BlobWriter};

/// A [`BodySink`] whose destination is the blob store.
///
/// See the [module documentation](self) for the contract.
#[derive(Debug)]
pub struct BlobSink {
    /// `None` once the blob has been published, which is what makes `finish`
    /// exactly-once without a separate flag.
    writer: Option<BlobWriter>,
    blob: Option<BlobId>,
    bytes: u64,
}

impl BlobSink {
    /// Opens a sink writing into `blobs`.
    ///
    /// # Errors
    ///
    /// [`BackendError::Storage`] if the temporary file cannot be created.
    pub fn new(blobs: &BlobStore) -> BackendResult<Self> {
        Ok(Self {
            writer: Some(blobs.writer().map_err(storage_error)?),
            blob: None,
            bytes: 0,
        })
    }

    /// The blob, once the fetch has completed.
    ///
    /// `None` until [`BodySink::finish`] has run, so a caller cannot mistake a
    /// fragment for a body — which is the whole point of the sink contract.
    pub fn finished_blob(&self) -> Option<BlobId> {
        self.blob.clone()
    }

    /// How many bytes have been written.
    ///
    /// Meaningful before `finish` too: it is what the backfill reports as
    /// progress, and what a cancelled fetch wasted.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

#[async_trait]
impl BodySink for BlobSink {
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()> {
        let writer = self.writer.as_mut().ok_or_else(|| BackendError::Protocol {
            reason: "bytes arrived after the fetch was finished".to_owned(),
        })?;
        writer.write(bytes).map_err(storage_error)?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }

    async fn finish(&mut self) -> BackendResult<()> {
        let writer = self.writer.take().ok_or_else(|| BackendError::Protocol {
            reason: "the fetch was finished twice".to_owned(),
        })?;
        self.blob = Some(writer.finish().map_err(storage_error)?);
        Ok(())
    }
}

/// A storage failure, in the vocabulary the backend seam speaks.
///
/// The sink sits on the protocol side of the seam, so a disk that is full has
/// to arrive at the caller as a fetch failure rather than as a type from a
/// crate the caller does not import.
fn storage_error(error: postio_storage::Error) -> BackendError {
    BackendError::Protocol {
        reason: format!("the blob store refused the fetched bytes: {error}"),
    }
}
