//! XChaCha20-Poly1305 over a blob's payload, one 64 KiB chunk at a time.
//!
//! # Why chunks and not one seal
//!
//! ADR 0014 asks for a per-blob AEAD; the blob store's own promise is that a
//! 30 MiB attachment never exists whole in memory. Those two pull against each
//! other: a single AEAD seal cannot be verified until its last byte has been
//! read, so honouring the tag would mean buffering the attachment — and
//! handing bytes out before the tag verified would mean the tag protects
//! nothing.
//!
//! The way out is the standard one: split the payload into chunks, seal each
//! under a nonce derived from one random per-blob nonce, and mark the last
//! chunk as last. That is the STREAM construction (Hoang, Reyhanitabar, Rogaway
//! and Vizár, 2015), and it buys three properties a naive chunking does not:
//!
//! * **Reordering is caught**, because the chunk's index is in its nonce.
//! * **Truncation is caught**, because the final chunk is sealed under a nonce
//!   that says so and no earlier chunk can be mistaken for it.
//! * **Chunks cannot be moved between blobs**, because the random prefix
//!   differs.
//!
//! # Why it is written out here
//!
//! The RustCrypto `aead` crate carried a `stream` module until 0.6 dropped it;
//! `chacha20poly1305` 0.11 sits on that version. Pinning the 0.10 line to get
//! the module back would put a second `aead`, a second `chacha20` and a second
//! `poly1305` in the graph, on a maintenance branch, to avoid the twenty lines
//! below. What those lines do is lay out a nonce — the cipher itself is still
//! the library's — and [`chunk_nonce`] is pinned by a test for exactly the
//! reason [`crate::key::Purpose::context`] is: it is format.
//!
//! # The nonce layout
//!
//! ```text
//! ┌───────────────── 24 bytes ─────────────────┐
//! │ prefix (19, random) │ index (4, BE) │ last │
//! └────────────────────────────────────────────┘
//! ```
//!
//! The prefix is what the header stores; the other five bytes are recomputed
//! per chunk on both sides. XChaCha20's 24-byte nonce is what makes a *random*
//! prefix safe here — with a 12-byte nonce there would not be room for a
//! random part large enough to pick without coordination.

use std::io::{self, Read, Write};

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

use crate::key::Subkey;

/// Poly1305's tag, appended to every chunk's ciphertext.
pub(crate) const TAG_LEN: usize = 16;

/// Bytes of the per-blob nonce that are stored in the header.
///
/// The remaining five are the chunk index and the last-block flag; see the
/// [module documentation](self).
pub(crate) const NONCE_PREFIX_LEN: usize = 19;

/// Plaintext bytes per chunk.
///
/// The 64 KiB the blob store already reads and writes in, so sealing adds no
/// buffering of its own: one chunk in flight, not a blob.
pub(crate) const CHUNK: usize = 64 * 1024;

/// A sealed chunk on disk: a full plaintext chunk plus its tag.
const SEALED_CHUNK: usize = CHUNK + TAG_LEN;

/// A fresh per-blob nonce prefix from the operating system's RNG.
///
/// Random rather than counted: there is no store-wide counter that survives a
/// crash, and a repeated nonce under the same key is the one mistake this
/// cipher does not forgive. 19 random bytes make a collision unreachable.
pub(crate) fn fresh_nonce_prefix() -> [u8; NONCE_PREFIX_LEN] {
    let mut prefix = [0u8; NONCE_PREFIX_LEN];
    getrandom::fill(&mut prefix).expect("the operating system has no random number source");
    prefix
}

/// The nonce chunk `index` of a blob is sealed under.
///
/// **This is format.** Change the layout and every stored blob stops opening;
/// a test pins it for that reason.
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

/// The error a reader gives back when a blob does not authenticate.
///
/// One sentence, one place: `blob.rs` turns it into
/// [`crate::Error::UnreadableBlob`], and the word "authentic" in it is what
/// tells a person the file was changed rather than merely unreadable.
fn not_authentic() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "this blob is not authentic: its contents were changed, truncated or \
         written under a different key",
    )
}

/// Seals everything written to it and passes the ciphertext on to `inner`.
///
/// Sits *below* compression: the payload reaching here is already whatever the
/// codec made of it, which is the `id, then compress, then encrypt` ordering
/// ADR 0017 and ADR 0014 agreed on.
///
/// Nothing is sealed until a whole chunk has arrived or [`Sealer::finish`] is
/// called, so a caller that writes a byte at a time still produces the same
/// file as one that writes 64 KiB at a time.
pub(crate) struct Sealer<W: Write> {
    cipher: XChaCha20Poly1305,
    prefix: [u8; NONCE_PREFIX_LEN],
    index: u32,
    /// Plaintext waiting for its chunk to fill. Never longer than [`CHUNK`].
    pending: Vec<u8>,
    inner: W,
}

impl<W: Write> Sealer<W> {
    /// A sealer over a prefix the caller already wrote into the file's header.
    ///
    /// The prefix is the caller's to mint ([`fresh_nonce_prefix`]) because it
    /// has to reach the header before the sealer takes the file.
    pub(crate) fn with_prefix(inner: W, key: &Subkey, prefix: [u8; NONCE_PREFIX_LEN]) -> Self {
        Self {
            cipher: cipher(key),
            prefix,
            index: 0,
            pending: Vec::with_capacity(CHUNK),
            inner,
        }
    }

    /// Seals `self.pending` and writes it out.
    fn seal_pending(&mut self, last: bool) -> io::Result<()> {
        let nonce = chunk_nonce(&self.prefix, self.index, last);
        let tag = self
            .cipher
            .encrypt_inout_detached(&nonce, &[], self.pending.as_mut_slice().into())
            .map_err(|_| io::Error::other("a blob chunk could not be encrypted"))?;
        self.inner.write_all(&self.pending)?;
        self.inner.write_all(&tag)?;
        self.pending.clear();
        // 2^32 chunks is 256 TiB; reaching it means something is wrong rather
        // than that somebody has a very large attachment. Refusing beats
        // wrapping the index and repeating a nonce.
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| io::Error::other("this blob is too large for the container format"))?;
        Ok(())
    }

    /// Seals whatever is left, marks it as the last chunk, and hands `inner`
    /// back.
    ///
    /// Always writes at least one chunk, even for an empty payload: a blob
    /// whose file held no chunks at all could not be told from a truncated
    /// one.
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
            // A full chunk is sealed only once there is definitely more to
            // come. Sealing eagerly on the 64 KiB boundary would commit a
            // payload of exactly one chunk to a non-last chunk plus an empty
            // last one -- a different file, and `finish` is what decides
            // which chunk is the last.
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
        // Deliberately not sealing: a flush that closed the chunk early would
        // change the file for a caller that only wanted its bytes pushed
        // along, and `zstd`'s encoder flushes on its own schedule.
        self.inner.flush()
    }
}

/// Unseals a payload as it is read, verifying each chunk before any of it
/// leaves.
///
/// A chunk that does not authenticate is an error, never short bytes: the
/// whole point is that a caller cannot accidentally read a blob somebody
/// edited.
pub(crate) struct Opener<R: Read> {
    cipher: XChaCha20Poly1305,
    prefix: [u8; NONCE_PREFIX_LEN],
    index: u32,
    inner: R,
    /// The next sealed chunk, read ahead so the one before it can be told it
    /// is not the last.
    lookahead: Option<Vec<u8>>,
    /// Verified plaintext not yet handed out, and how much of it has been.
    plain: Vec<u8>,
    taken: usize,
    /// Set once the chunk marked last has been opened.
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

    /// Opens the next chunk into `self.plain`, or reports the end.
    ///
    /// A chunk is the last one when nothing follows it, which is why this
    /// always holds one chunk of lookahead: the last-block flag is in the
    /// nonce, so the reader has to know before it can decrypt.
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
        // Too short to be a chunk at all: a file cut off inside its first tag,
        // or an empty payload where the format guarantees one sealed chunk.
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

/// Reads one sealed chunk, or as much of it as the file has left.
///
/// Fills right up to [`SEALED_CHUNK`] rather than trusting one `read`: a chunk
/// split across two reads would otherwise look like the end of the file, and
/// the chunk before it would be verified under the wrong nonce.
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

    /// The nonce layout is on disk in every blob a user owns. Pinned by value,
    /// the same way `Purpose::context` is.
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
        // The boundary the eager-seal bug lives on: seal on a full buffer and
        // this payload gets a non-last chunk followed by an empty last one,
        // which is a different file and a different amount of disk.
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
