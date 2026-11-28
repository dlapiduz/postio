//! Fetched bytes stream into a `BodySink`, so a 40 MB attachment never exists whole in memory.

use async_trait::async_trait;

use super::BackendResult;

/// Chunk boundaries carry no meaning: do not assume they fall on lines or base64 quads.
#[async_trait]
pub trait BodySink: Send {
    async fn chunk(&mut self, bytes: &[u8]) -> BackendResult<()>;

    /// A sink that was not called here did not receive a whole body: discard what was written.
    async fn finish(&mut self) -> BackendResult<()> {
        Ok(())
    }
}

/// For tests; counts chunks so a test can assert that a large body streamed.
#[derive(Clone, Debug, Default)]
pub struct VecSink {
    bytes: Vec<u8>,
    chunks: usize,
    finished: bool,
}

impl VecSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn chunks(&self) -> usize {
        self.chunks
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

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

#[derive(Clone, Copy, Debug, Default)]
pub struct CountingSink {
    bytes: u64,
}

impl CountingSink {
    pub fn new() -> Self {
        Self::default()
    }

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
