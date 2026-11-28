//! XChaCha20-Poly1305 over a payload in 64 KiB chunks (STREAM construction), so a 30 MiB attachment
//! is verified without being buffered whole.
//! Each chunk's nonce is a random per-blob prefix plus its index and a last flag, which catches
//! reordering, truncation and chunks moved between blobs.
//! Written out here because `aead` 0.6 dropped its `stream` module; `chunk_nonce` is pinned by a
//! test as format.

use std::io::{self, Read, Write};

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::key::Subkey;

pub(crate) const TAG_LEN: usize = 16;

/// The remaining five bytes are the chunk index and the last-block flag.
pub(crate) const NONCE_PREFIX_LEN: usize = 19;

pub(crate) const CHUNK: usize = 64 * 1024;

const SEALED_CHUNK: usize = CHUNK + TAG_LEN;

/// Random, not counted: no store-wide counter survives a crash, and a repeated nonce under one key
/// is unforgivable.
pub(crate) fn fresh_nonce_prefix() -> [u8; NONCE_PREFIX_LEN] {
    let mut prefix = [0u8; NONCE_PREFIX_LEN];
    getrandom::fill(&mut prefix).expect("the operating system has no random number source");
    prefix
}

/// **Format.** Changing the layout stops every stored blob opening; a test pins it.
fn chunk_nonce(prefix: &[u8; NONCE_PREFIX_LEN], index: u32, last: bool) -> XNonce {
    let mut nonce = [0u8; 24];
    nonce[..NONCE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[NONCE_PREFIX_LEN..NONCE_PREFIX_LEN + 4].copy_from_slice(&index.to_be_bytes());
    nonce[23] = u8::from(last);
    XNonce::from(nonce)
}

fn cipher(key: &Subkey) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.expose().into())
}

/// `blob.rs` turns it into [`crate::Error::UnreadableBlob`]; "authentic" tells a person the file
/// was changed, not merely unreadable.
fn not_authentic() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "this blob is not authentic: its contents were changed, truncated or \
         written under a different key",
    )
}

/// Sits below compression: the `id, then compress, then encrypt` ordering (ADR 0017, ADR 0014).
///
/// Seals only on a full chunk or [`Sealer::finish`], so write granularity never changes the file.
pub(crate) struct Sealer<W: Write> {
    cipher: XChaCha20Poly1305,
    prefix: [u8; NONCE_PREFIX_LEN],
    index: u32,
    pending: Vec<u8>,
    inner: W,
}

impl<W: Write> Sealer<W> {
    /// The caller mints the prefix ([`fresh_nonce_prefix`]) because it must reach the header first.
    pub(crate) fn with_prefix(inner: W, key: &Subkey, prefix: [u8; NONCE_PREFIX_LEN]) -> Self {
        Self {
            cipher: cipher(key),
            prefix,
            index: 0,
            pending: Vec::with_capacity(CHUNK),
            inner,
        }
    }

    fn seal_pending(&mut self, last: bool) -> io::Result<()> {
        let nonce = chunk_nonce(&self.prefix, self.index, last);
        let tag = self
            .cipher
            .encrypt_inout_detached(&nonce, &[], self.pending.as_mut_slice().into())
            .map_err(|_| io::Error::other("a blob chunk could not be encrypted"))?;
        self.inner.write_all(&self.pending)?;
        self.inner.write_all(&tag)?;
        self.pending.clear();
        // Refuse rather than wrap the index and repeat a nonce (2^32 chunks is 256 TiB).
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| io::Error::other("this blob is too large for the container format"))?;
        Ok(())
    }

    /// Always writes at least one chunk, even for an empty payload, so a chunkless file reads as
    /// truncated.
    pub(crate) fn finish(mut self) -> io::Result<W> {
        self.seal_pending(true)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for Sealer<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut rest = bytes;
        while !rest.is_empty() {
            // Seal a full chunk only once more is coming: `finish` decides which chunk is last, and
            // eager sealing would turn an exactly-one-chunk payload into a different file.
            if self.pending.len() == CHUNK {
                self.seal_pending(false)?;
            }
            let room = CHUNK - self.pending.len();
            let take = room.min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Not sealing: an early-closed chunk would change the file, and zstd flushes on its own
        // schedule.
        self.inner.flush()
    }
}

/// A chunk that fails to authenticate is an error, never short bytes.
pub(crate) struct Opener<R: Read> {
    cipher: XChaCha20Poly1305,
    prefix: [u8; NONCE_PREFIX_LEN],
    index: u32,
    inner: R,
    /// Read ahead so the chunk before it knows it is not the last.
    lookahead: Option<Vec<u8>>,
    plain: Vec<u8>,
    taken: usize,
    done: bool,
}

impl<R: Read> Opener<R> {
    pub(crate) fn new(inner: R, key: &Subkey, prefix: [u8; NONCE_PREFIX_LEN]) -> Self {
        Self {
            cipher: cipher(key),
            prefix,
            index: 0,
            inner,
            lookahead: None,
            plain: Vec::new(),
            taken: 0,
            done: false,
        }
    }

    /// Holds one chunk of lookahead: the last-block flag is in the nonce, so the reader must know
    /// before decrypting.
    fn open_next(&mut self) -> io::Result<bool> {
        if self.done {
            return Ok(false);
        }
        let mut current = match self.lookahead.take() {
            Some(chunk) => chunk,
            None => read_chunk(&mut self.inner)?,
        };
        let following = read_chunk(&mut self.inner)?;
        let last = following.is_empty();
        if !last {
            self.lookahead = Some(following);
        }
        // Too short to be a chunk: cut off inside its first tag, or an empty payload.
        if current.len() < TAG_LEN {
            return Err(not_authentic());
        }
        let tag: [u8; TAG_LEN] = current
            .split_off(current.len() - TAG_LEN)
            .try_into()
            .expect("exactly a tag was split off");
        let nonce = chunk_nonce(&self.prefix, self.index, last);
        self.cipher
            .decrypt_inout_detached(&nonce, &[], current.as_mut_slice().into(), (&tag).into())
            .map_err(|_| not_authentic())?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| io::Error::other("this blob is too large for the container format"))?;
        self.plain = current;
        self.taken = 0;
        self.done = last;
        Ok(true)
    }
}

impl<R: Read> Read for Opener<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.taken == self.plain.len() {
            if !self.open_next()? {
                return Ok(0);
            }
        }
        let take = (self.plain.len() - self.taken).min(out.len());
        out[..take].copy_from_slice(&self.plain[self.taken..self.taken + take]);
        self.taken += take;
        Ok(take)
    }
}

/// Fills up to [`SEALED_CHUNK`]: a chunk split across two reads would look like end of file and the
/// previous chunk would be verified under the wrong nonce.
fn read_chunk(source: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0u8; SEALED_CHUNK];
    let mut filled = 0;
    while filled < SEALED_CHUNK {
        match source.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{BlobKeys, StoreKey};

    fn key() -> Subkey {
        BlobKeys::derive(&StoreKey::from_bytes([0x11; 32]))
            .content()
            .clone()
    }

    #[test]
    fn the_nonce_layout_is_pinned() {
        let prefix = [0xab; NONCE_PREFIX_LEN];

        let first = chunk_nonce(&prefix, 0, false);
        assert_eq!(&first[..NONCE_PREFIX_LEN], &prefix);
        assert_eq!(&first[NONCE_PREFIX_LEN..], &[0, 0, 0, 0, 0]);

        let later = chunk_nonce(&prefix, 0x0102_0304, true);
        assert_eq!(&later[NONCE_PREFIX_LEN..], &[1, 2, 3, 4, 1]);

        assert_ne!(
            chunk_nonce(&prefix, 7, false),
            chunk_nonce(&prefix, 7, true),
            "the last-block flag must change the nonce, or truncation is undetectable"
        );
    }

    fn round_trip(payload: &[u8], write_in: usize) -> Vec<u8> {
        let key = key();
        let prefix = fresh_nonce_prefix();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key, prefix);
        for piece in payload.chunks(write_in.max(1)) {
            sealer.write_all(piece).expect("seal");
        }
        let sealed = sealer.finish().expect("finish");

        let mut out = Vec::new();
        Opener::new(sealed.as_slice(), &key, prefix)
            .read_to_end(&mut out)
            .expect("open");
        out
    }

    #[test]
    fn payloads_round_trip_whatever_the_write_sizes_were() {
        for len in [0usize, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK] {
            let payload: Vec<u8> = (0..len).map(|n| (n % 251) as u8).collect();
            for write_in in [1usize, 7, CHUNK, CHUNK * 4] {
                assert_eq!(
                    round_trip(&payload, write_in),
                    payload,
                    "{len} bytes written {write_in} at a time"
                );
            }
        }
    }

    #[test]
    fn a_payload_of_exactly_one_chunk_is_one_sealed_chunk() {
        // Boundary of the eager-seal bug: sealing a full buffer gives a non-last chunk plus an
        // empty last one.
        let key = key();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key, fresh_nonce_prefix());
        sealer.write_all(&vec![7u8; CHUNK]).expect("seal");
        assert_eq!(sealer.finish().expect("finish").len(), CHUNK + TAG_LEN);
    }

    #[test]
    fn a_flipped_byte_anywhere_is_refused() {
        let key = key();
        let payload: Vec<u8> = (0..(2 * CHUNK + 9)).map(|n| (n % 253) as u8).collect();
        let prefix = fresh_nonce_prefix();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key, prefix);
        sealer.write_all(&payload).expect("seal");
        let sealed = sealer.finish().expect("finish");

        for at in [0usize, 100, CHUNK, 2 * (CHUNK + TAG_LEN), sealed.len() - 1] {
            let mut damaged = sealed.clone();
            damaged[at] ^= 0x01;
            let mut out = Vec::new();
            assert!(
                Opener::new(damaged.as_slice(), &key, prefix)
                    .read_to_end(&mut out)
                    .is_err(),
                "a byte flipped at {at} was read out as if it were mail"
            );
        }
    }

    #[test]
    fn dropping_the_last_chunk_is_refused_rather_than_read_short() {
        let key = key();
        let payload: Vec<u8> = (0..(2 * CHUNK)).map(|n| (n % 249) as u8).collect();
        let prefix = fresh_nonce_prefix();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key, prefix);
        sealer.write_all(&payload).expect("seal");
        let sealed = sealer.finish().expect("finish");

        let truncated = &sealed[..CHUNK + TAG_LEN];
        let mut out = Vec::new();
        assert!(
            Opener::new(truncated, &key, prefix)
                .read_to_end(&mut out)
                .is_err(),
            "a truncated payload was handed back as a whole one"
        );
    }

    #[test]
    fn swapping_two_chunks_is_refused() {
        let key = key();
        let payload: Vec<u8> = (0..(3 * CHUNK)).map(|n| (n % 247) as u8).collect();
        let prefix = fresh_nonce_prefix();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key, prefix);
        sealer.write_all(&payload).expect("seal");
        let sealed = sealer.finish().expect("finish");

        let mut reordered = sealed.clone();
        let (first, rest) = reordered.split_at_mut(CHUNK + TAG_LEN);
        first.swap_with_slice(&mut rest[..CHUNK + TAG_LEN]);

        let mut out = Vec::new();
        assert!(
            Opener::new(reordered.as_slice(), &key, prefix)
                .read_to_end(&mut out)
                .is_err(),
            "two chunks were swapped and the blob still opened"
        );
    }

    #[test]
    fn another_key_does_not_open_it() {
        let payload = b"the frobnicator arrives on Thursday";
        let prefix = fresh_nonce_prefix();
        let mut sealer = Sealer::with_prefix(Vec::new(), &key(), prefix);
        sealer.write_all(payload).expect("seal");
        let sealed = sealer.finish().expect("finish");

        let other = BlobKeys::derive(&StoreKey::from_bytes([0x22; 32]))
            .content()
            .clone();
        let mut out = Vec::new();
        assert!(
            Opener::new(sealed.as_slice(), &other, prefix)
                .read_to_end(&mut out)
                .is_err()
        );
    }
}
