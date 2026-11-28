//! A `BodySink` that writes straight into the blob store: no `Vec` doubling, the file on disk is
//! the buffer (ADR 0017).
//! Lives here, not in `postio-storage`, because joining protocol to storage is this crate's job.
//! `finish` publishes the blob; an unfinished sink publishes nothing and exposes no id.

use async_trait::async_trait;
use postio_account::backend::{BackendError, BackendResult, BodySink};
use postio_model::BlobId;
use postio_storage::{BlobStore, BlobWriter};

#[derive(Debug)]
pub struct BlobSink {
    /// `None` once the blob has been published, which makes `finish` exactly-once.
    writer: Option<BlobWriter>,
    blob: Option<BlobId>,
    bytes: u64,
}

impl BlobSink {
    pub fn new(blobs: &BlobStore) -> BackendResult<Self> {
        Ok(Self {
            writer: Some(blobs.writer().map_err(storage_error)?),
            blob: None,
            bytes: 0,
        })
    }

    /// `None` until `finish` has run, so a fragment cannot be mistaken for a body.
    pub fn finished_blob(&self) -> Option<BlobId> {
        self.blob.clone()
    }

    /// Bytes written so far, also before `finish`.
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

/// A full disk must reach the caller as a fetch failure, not a type from a crate it does not
/// import.
fn storage_error(error: postio_storage::Error) -> BackendError {
    BackendError::Protocol {
        reason: format!("the blob store refused the fetched bytes: {error}"),
    }
}
