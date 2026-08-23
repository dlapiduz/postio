//! Where fetched bytes go.
//!
//! A body fetch does not return a `Vec<u8>`. It writes into a sink as the
//! bytes arrive, so a 40 MB attachment lands in the blob store without ever
//! existing whole in memory — and so the same call can report progress, hash
//! as it goes, or be abandoned halfway without having paid for the rest.
//!
//! `postio-storage` implements this over the blob store. [`VecSink`] is the
//! one for tests.

use async_trait::async_trait;

use super::BackendResult;

/// A destination for the bytes of a message or a MIME part.
///
/// Chunk boundaries carry no meaning: they are wherever the transport happened
/// to split, and an implementation must not assume they fall on lines, on
/// base64 quads, or anywhere else.
#[async_trait]
pub trait BodySink: Send {
    /// Accepts the next run of bytes.
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()>;

    /// Called once, after the last chunk, when the fetch completed.
    ///
    /// A sink that is not called here did not receive a whole body: the fetch
    /// failed or was cancelled, and whatever was written must be discarded
    /// rather than treated as a short message.
    async fn finish(&mut self) -> BackendResult<()> {
        Ok(())
    }
}

/// A [`BodySink`] that collects into a `Vec<u8>`.
///
/// For tests and for the rare small fetch where streaming buys nothing. It
/// counts chunks as well as bytes, which is how a test asserts that a large
/// body actually streamed rather than arriving in one buffer.
#[derive(Clone, Debug, Default)]
pub struct VecSink {
    bytes: Vec<u8>,
    chunks: usize,
    finished: bool,
}

impl VecSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes written so far.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// How many bytes were written.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether nothing was written.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// How many separate chunks arrived.
    pub fn chunks(&self) -> usize {
        self.chunks
    }

    /// Whether the fetch ran to completion.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Takes the collected bytes.
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

#[async_trait]
impl BodySink for VecSink {
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()> {
        self.bytes.extend_from_slice(bytes);
        self.chunks += 1;
        Ok(())
    }

    async fn finish(&mut self) -> BackendResult<()> {
        self.finished = true;
        Ok(())
    }
}

/// A [`BodySink`] that keeps only the byte count.
///
/// Useful for measuring a fetch, and for the "download it to prove we can"
/// path in a live test that has no interest in the content.
#[derive(Clone, Copy, Debug, Default)]
pub struct CountingSink {
    bytes: u64,
}

impl CountingSink {
    /// A sink at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many bytes went past.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

#[async_trait]
impl BodySink for CountingSink {
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()> {
        self.bytes += bytes.len() as u64;
        Ok(())
    }
}

#[async_trait]
impl<T: BodySink + ?Sized> BodySink for &mut T {
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()> {
        (**self).chunk(bytes).await
    }

    async fn finish(&mut self) -> BackendResult<()> {
        (**self).finish().await
    }
}
