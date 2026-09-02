//! The container a blob file is written in.
//!
//! # Why there is a container at all
//!
//! A blob used to be its plaintext bytes and nothing else. That is a format
//! with no room in it: ADR 0017 wants compression, ADR 0014 wants a per-blob
//! nonce and an AEAD tag, and neither can be added to a bare file without
//! rewriting every blob a user owns. Under ADR 0016 that is their entire
//! mailbox, so a flag day here is measured in hours of somebody's disk.
//!
//! So blobs written from now on carry a fixed header saying what they are, and
//! readers dispatch on it. The reserved byte was where ADR 0014's encryption
//! field would land, and the version is what let it arrive without another
//! flag day. #301 spent both: the reserved byte is now the cipher, the version
//! is 2, and a random nonce prefix follows the fixed part.
//!
//! # Two versions, one of them read-only
//!
//! Version 1 is the compressed-but-unencrypted container this build no longer
//! writes. It is still *read*, because the pre-release migration (ADR 0014 Q4)
//! has to open the old store to re-encrypt it, and so does a development store
//! nobody has migrated yet. Version 2 is always encrypted; there is no cipher
//! value meaning "none" at that version, which is what keeps ADR 0014's
//! no-plaintext-fallback rule from being expressible here.
//!
//! # Legacy blobs are not migrated
//!
//! A file that does not begin with [`MAGIC`] is a blob from before this
//! existed: read it verbatim, for ever. There is nothing to gain by rewriting
//! it — the bytes are already correct, and a migration over a whole mailbox to
//! save a few percent is a bad trade against the chance of losing one.
//!
//! [`MAGIC`] begins with a NUL for that dispatch to be safe. A stored
//! plaintext is a raw RFC 5322 message, a decoded text or HTML body, or a file
//! the user attached; none of the first three can begin with a NUL byte, and
//! the fourth would have to match four more magic bytes and two valid
//! discriminants as well to be mistaken for a container.

use crate::blob::seal;
use crate::error::{Error, Result};

/// Marks a file as a container rather than bare plaintext.
///
/// Leading NUL on purpose; see the [module documentation](self).
pub(crate) const MAGIC: [u8; 5] = [0x00, b'P', b'B', b'L', b'B'];

/// The compressed, unencrypted container. Read, never written; see the
/// [module documentation](self).
pub(crate) const VERSION_PLAIN: u8 = 1;

/// The container this build writes: compressed and encrypted.
pub(crate) const VERSION: u8 = 2;

/// Bytes of header shared by both versions.
///
/// `MAGIC` (5) ‖ version (1) ‖ codec (1) ‖ dictionary id (4, little-endian)
/// ‖ cipher (1).
pub(crate) const FIXED_LEN: usize = MAGIC.len() + 7;

/// Bytes of header before an encrypted payload begins.
///
/// [`FIXED_LEN`] plus the per-blob nonce prefix. A version 1 container's
/// payload begins at `FIXED_LEN` instead.
pub(crate) const HEADER_LEN: usize = FIXED_LEN + seal::NONCE_PREFIX_LEN;

/// Cipher byte for a version 1 container, which has none.
const CIPHER_NONE: u8 = 0;

/// Cipher byte for XChaCha20-Poly1305 over 64 KiB chunks (`blob::seal`).
const CIPHER_XCHACHA20_POLY1305: u8 = 1;

/// Dictionary id meaning "compressed against no dictionary".
///
/// **This is the only value a blob will ever carry**, and the field stays
/// anyway.
///
/// It was reserved for compressing text against a trained dictionary. ADR 0020
/// moved the text into `messages` rows, where the dictionary lives as a row
/// beside it (`postio_storage::body`), and what is left in the blob store is
/// attachment payloads and raw `.eml`. Payloads are largely already compressed
/// — 8.9 GB of JPEG, PNG, PDF and ZIP on the reference account — and a
/// dictionary buys nothing on bytes that do not compress.
///
/// Removing the field would mean a container version bump and rewriting every
/// blob a user owns, which under ADR 0016 is their whole mailbox, to reclaim
/// four bytes per file. Reading it and refusing anything else costs nothing
/// and keeps the door open, which is what a version field is for.
pub(crate) const NO_DICTIONARY: u32 = 0;

/// Compression level for the incompressibility probe.
///
/// Deliberately lower than [`LEVEL`]: the probe is a yes/no question asked on
/// every write, and a cheap answer that is occasionally pessimistic costs far
/// less than an expensive one that is exactly right.
const PROBE_LEVEL: i32 = 1;

/// Compression level for stored blobs.
///
/// 3 is zstd's own default and the knee of the curve for text: most of the
/// ratio for a small fraction of the time the higher levels take. Mail is read
/// far more often than written, but zstd's decompression speed barely varies
/// with the level it was written at, so paying more here buys almost nothing.
pub(crate) const LEVEL: i32 = 3;

/// How a blob's payload is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    /// Stored as-is. What an already-compressed payload gets: 8.9 GB of the
    /// reference account is JPEG, PNG, PDF and ZIP, and running those through
    /// zstd spends CPU on every read and write to make them slightly bigger.
    None,
    /// zstd, no dictionary.
    Zstd,
}

impl Codec {
    fn as_byte(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Zstd => 1,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::None),
            1 => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// The bytes that precede a container's payload.
pub(crate) fn header(
    codec: Codec,
    dictionary: u32,
    nonce: &[u8; seal::NONCE_PREFIX_LEN],
) -> [u8; HEADER_LEN] {
    let mut bytes = [0u8; HEADER_LEN];
    bytes[..MAGIC.len()].copy_from_slice(&MAGIC);
    bytes[MAGIC.len()] = VERSION;
    bytes[MAGIC.len() + 1] = codec.as_byte();
    bytes[MAGIC.len() + 2..MAGIC.len() + 6].copy_from_slice(&dictionary.to_le_bytes());
    bytes[MAGIC.len() + 6] = CIPHER_XCHACHA20_POLY1305;
    bytes[FIXED_LEN..].copy_from_slice(nonce);
    bytes
}

/// What a file's opening bytes say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framing {
    /// A container: skip [`Container::payload_at`] bytes and decode with the
    /// codec it names, after unsealing it if it is encrypted.
    Container(Container),
    /// Bare plaintext from before the container existed.
    Legacy,
}

/// A container header, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Container {
    /// How the payload is encoded, under the encryption.
    pub(crate) codec: Codec,
    /// The per-blob nonce prefix, or `None` for a version 1 container.
    pub(crate) nonce: Option<[u8; seal::NONCE_PREFIX_LEN]>,
}

impl Container {
    /// Where the payload begins.
    pub(crate) fn payload_at(self) -> usize {
        if self.nonce.is_some() {
            HEADER_LEN
        } else {
            FIXED_LEN
        }
    }
}

/// Reads the framing of a file that begins with `start`.
///
/// `start` may be short — a blob smaller than a header is necessarily legacy.
///
/// # Errors
///
/// [`Error::UnreadableBlob`] if the file claims to be a container but names a
/// version or codec this build does not know. That is a store written by a
/// newer Postio, and guessing would hand back wrong bytes under a digest that
/// promises otherwise; refusing is the only safe answer.
pub(crate) fn framing_of(start: &[u8]) -> Result<Framing> {
    if start.len() < FIXED_LEN || start[..MAGIC.len()] != MAGIC {
        return Ok(Framing::Legacy);
    }
    let version = start[MAGIC.len()];
    if version != VERSION && version != VERSION_PLAIN {
        return Err(Error::UnreadableBlob {
            reason: format!("blob container version {version} is newer than this build reads"),
        });
    }
    let codec = start[MAGIC.len() + 1];
    let codec = Codec::from_byte(codec).ok_or_else(|| Error::UnreadableBlob {
        reason: format!("blob codec {codec} is not one this build knows"),
    })?;
    let dictionary = u32::from_le_bytes(
        start[MAGIC.len() + 2..MAGIC.len() + 6]
            .try_into()
            .expect("four bytes"),
    );
    if dictionary != NO_DICTIONARY {
        return Err(Error::UnreadableBlob {
            reason: format!(
                "blob was compressed against dictionary {dictionary}, which this build does not have"
            ),
        });
    }
    let nonce = nonce_of(start, version, start[MAGIC.len() + 6])?;
    Ok(Framing::Container(Container { codec, nonce }))
}

/// The nonce prefix a container carries, checked against its version.
///
/// The two are not independent: version 1 has no cipher and no nonce, version
/// 2 always has both. A file claiming any other combination was written by
/// something this build does not understand, and guessing would hand back
/// wrong bytes under a digest that promises otherwise.
fn nonce_of(start: &[u8], version: u8, cipher: u8) -> Result<Option<[u8; seal::NONCE_PREFIX_LEN]>> {
    match (version, cipher) {
        (VERSION_PLAIN, CIPHER_NONE) => Ok(None),
        (VERSION, CIPHER_XCHACHA20_POLY1305) => {
            let Some(bytes) = start.get(FIXED_LEN..HEADER_LEN) else {
                return Err(Error::UnreadableBlob {
                    reason: "this blob stops inside its own header".to_owned(),
                });
            };
            Ok(Some(bytes.try_into().expect("the slice is that long")))
        }
        (_, cipher) => Err(Error::UnreadableBlob {
            reason: format!(
                "blob cipher {cipher} is not one a version {version} container may carry"
            ),
        }),
    }
}

/// Whether `sample` is worth compressing.
///
/// Decided from the first chunk rather than by compressing everything and
/// comparing: the whole point is not to spend CPU on the payloads that will
/// not shrink. A sample zstd cannot get below 90% of its size is taken as
/// already compressed.
pub(crate) fn looks_compressible(sample: &[u8]) -> bool {
    // Too small to tell, and too small to matter either way. Compressing keeps
    // small attachments and short raw messages on one path.
    if sample.len() < 512 {
        return true;
    }
    let Ok(trial) = zstd::bulk::compress(sample, PROBE_LEVEL) else {
        return false;
    };
    trial.len() * 10 < sample.len() * 9
}
