//! `COMPRESS=DEFLATE` (RFC 4978): raw deflate under the protocol.
//!
//! The protocol crates are sans-I/O, so compression is a property of the
//! byte stream they are pumped against, not of a coroutine. After the server
//! accepts `COMPRESS DEFLATE`, [`DeflateStream`] replaces the stream the
//! session was driving and everything above it carries on unchanged.
//!
//! Two rules from the RFC make this more than a `GzEncoder` wrapper:
//!
//! * The stream is raw deflate, with no zlib header or checksum.
//! * Every write ends on a sync flush. Without it the compressor holds back
//!   the tail of a command in its window, the server never sees it, and the
//!   exchange hangs. `IDLE` and `DONE` are the first things to notice.

use std::io;

use async_trait::async_trait;
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

use crate::{Stream, TransportError};

/// How much compressed input is read from the wire at a time.
const WIRE_CHUNK: usize = 16 * 1024;

/// A raw-deflate encoder and decoder, with no I/O.
///
/// Shared by [`DeflateStream`] and the in-process test server, so the two
/// ends of a test agree with each other only because both follow the RFC.
pub struct DeflateCodec {
    encoder: Compress,
    decoder: Decompress,
    /// Compressed bytes read off the wire and not yet inflated.
    pending: Vec<u8>,
    consumed: usize,
}

impl std::fmt::Debug for DeflateCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeflateCodec")
            .field("pending", &(self.pending.len() - self.consumed))
            .finish_non_exhaustive()
    }
}

impl Default for DeflateCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl DeflateCodec {
    /// A fresh pair of raw (header-less) deflate states.
    pub fn new() -> Self {
        Self {
            encoder: Compress::new(Compression::fast(), false),
            decoder: Decompress::new(false),
            pending: Vec::new(),
            consumed: 0,
        }
    }

    /// Compresses `plain` and ends it on a sync flush, so the peer can
    /// inflate every byte of it without waiting for more.
    pub fn deflate(&mut self, plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(plain.len() / 2 + 64);
        let start = self.encoder.total_in();
        loop {
            let done = usize::try_from(self.encoder.total_in() - start).unwrap_or(usize::MAX);
            if out.capacity() == out.len() {
                out.reserve(plain.len().max(64));
            }
            self.encoder
                .compress_vec(&plain[done..], &mut out, FlushCompress::Sync)
                .expect("raw deflate cannot fail on in-memory input");
            let done = usize::try_from(self.encoder.total_in() - start).unwrap_or(usize::MAX);
            // Everything is consumed and the flush marker did not fill the
            // buffer, so nothing is left inside the compressor.
            if done == plain.len() && out.len() < out.capacity() {
                return out;
            }
        }
    }

    /// Queues compressed bytes that arrived from the peer.
    pub fn push(&mut self, wire: &[u8]) {
        if self.consumed == self.pending.len() {
            self.pending.clear();
            self.consumed = 0;
        }
        self.pending.extend_from_slice(wire);
    }

    /// Inflates queued bytes into `out`. `Ok(0)` means more input is needed.
    ///
    /// # Errors
    ///
    /// Corrupt deflate data, or the peer ending the deflate stream.
    pub fn inflate(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            // Called even with no input queued: the decoder takes input into
            // its window ahead of handing the output back.
            let (in_before, out_before) = (self.decoder.total_in(), self.decoder.total_out());
            let status = self
                .decoder
                .decompress(&self.pending[self.consumed..], out, FlushDecompress::Sync)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let taken = usize::try_from(self.decoder.total_in() - in_before).unwrap_or(0);
            self.consumed += taken;
            let produced = usize::try_from(self.decoder.total_out() - out_before).unwrap_or(0);
            if produced > 0 {
                return Ok(produced);
            }
            if matches!(status, Status::StreamEnd) {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the peer ended the compressed stream",
                ));
            }
            // Input consumed without output (a flush marker): try the rest.
            // Nothing consumed and nothing produced: starved.
            if taken == 0 {
                return Ok(0);
            }
        }
    }
}

/// A [`Stream`] whose bytes are raw deflate on the wire.
#[derive(Debug)]
pub struct DeflateStream {
    inner: Box<dyn Stream>,
    codec: DeflateCodec,
}

impl DeflateStream {
    /// Wraps `inner`, which must be the stream on which the server has just
    /// answered `COMPRESS DEFLATE` with `OK` and nothing else.
    pub fn new(inner: Box<dyn Stream>) -> Self {
        Self {
            inner,
            codec: DeflateCodec::new(),
        }
    }
}

impl DeflateStream {
    /// Replaces the stream in `slot` with itself wrapped in deflate.
    pub fn wrap(slot: &mut Box<dyn Stream>) {
        let inner = std::mem::replace(slot, Box::new(Vacant));
        *slot = Box::new(Self::new(inner));
    }
}

/// What [`DeflateStream::wrap`] leaves in the slot for the instant between
/// taking the old stream out and putting the new one in.
#[derive(Debug)]
struct Vacant;

#[async_trait]
impl Stream for Vacant {
    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
        Err(TransportError::Closed)
    }

    async fn write_all(&mut self, _bytes: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::Closed)
    }

    async fn upgrade_tls(self: Box<Self>, _host: &str) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Closed)
    }

    fn is_encrypted(&self) -> bool {
        false
    }
}

#[async_trait]
impl Stream for DeflateStream {
    /// Cancel-safe in the way the IDLE loop relies on: the only await is the
    /// inner read, and whatever it returned is queued before anything else.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let mut wire = [0u8; WIRE_CHUNK];
        loop {
            let inflated = self
                .codec
                .inflate(buf)
                .map_err(|error| crate::map_io("reading from the server", error))?;
            if inflated > 0 {
                return Ok(inflated);
            }
            let read = self.inner.read(&mut wire).await?;
            self.codec.push(&wire[..read]);
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let wire = self.codec.deflate(bytes);
        self.inner.write_all(&wire).await
    }

    async fn upgrade_tls(self: Box<Self>, _host: &str) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Unsupported(
            "STARTTLS was requested on a compressed connection".to_owned(),
        ))
    }

    fn is_encrypted(&self) -> bool {
        self.inner.is_encrypted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_one_codec_writes_the_other_reads_back_after_every_flush() {
        let mut client = DeflateCodec::new();
        let mut server = DeflateCodec::new();
        let mut out = [0u8; 4096];

        for message in ["a1 IDLE\r\n", "DONE\r\n", &"x".repeat(100_000)] {
            let wire = client.deflate(message.as_bytes());
            server.push(&wire);
            let mut got = Vec::new();
            loop {
                let n = server.inflate(&mut out).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&out[..n]);
            }
            assert_eq!(got, message.as_bytes(), "a flushed write must be whole");
        }
    }

    #[test]
    fn output_the_decoder_still_holds_is_not_mistaken_for_a_wait_for_input() {
        // The decoder takes all of its input into its window before it has
        // handed back all of its output, so "no input left" does not mean
        // "nothing to read". Stopping there hangs a large fetch.
        let text = "The quarterly figures are attached below, as agreed.\r\n".repeat(4_000);
        let wire = DeflateCodec::new().deflate(text.as_bytes());
        let mut codec = DeflateCodec::new();
        codec.push(&wire);

        let mut got = Vec::new();
        let mut out = [0u8; 1024];
        loop {
            let n = codec.inflate(&mut out).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&out[..n]);
        }
        assert_eq!(got.len(), text.len());
    }

    #[test]
    fn the_wire_form_is_raw_deflate_with_no_zlib_header() {
        let wire = DeflateCodec::new().deflate(b"a1 NOOP\r\n");
        // A zlib stream starts 0x78; a sync-flushed raw stream ends 00 00 FF FF.
        assert_ne!(wire[0], 0x78);
        assert_eq!(&wire[wire.len() - 4..], [0x00, 0x00, 0xFF, 0xFF]);
    }
}
