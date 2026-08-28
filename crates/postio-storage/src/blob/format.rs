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
//! So blobs written from now on carry a fixed eight-byte header saying what
//! they are, and readers dispatch on it. The reserved byte is where ADR 0014's
//! encryption fields land, and the version is what lets that arrive without
//! another flag day.
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

use crate::error::{Error, Result};

/// Marks a file as a container rather than bare plaintext.
///
/// Leading NUL on purpose; see the [module documentation](self).
pub(crate) const MAGIC: [u8; 5] = [0x00, b'P', b'B', b'L', b'B'];

/// The only container version so far.
pub(crate) const VERSION: u8 = 1;

/// Bytes of header before the payload begins.
///
/// `MAGIC` (5) ‖ version (1) ‖ codec (1) ‖ dictionary id (4, little-endian)
/// ‖ reserved (1).
pub(crate) const HEADER_LEN: usize = MAGIC.len() + 7;

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
pub(crate) fn header(codec: Codec, dictionary: u32) -> [u8; HEADER_LEN] {
    let mut bytes = [0u8; HEADER_LEN];
    bytes[..MAGIC.len()].copy_from_slice(&MAGIC);
    bytes[MAGIC.len()] = VERSION;
    bytes[MAGIC.len() + 1] = codec.as_byte();
    bytes[MAGIC.len() + 2..MAGIC.len() + 6].copy_from_slice(&dictionary.to_le_bytes());
    // The last byte stays zero: reserved for ADR 0014's encryption flags, so
    // adding them is a version bump rather than a change of shape.
    bytes
}

/// What a file's opening bytes say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framing {
    /// A container: skip [`HEADER_LEN`] bytes and decode with this codec,
    /// against this dictionary id.
    Container(Codec, u32),
    /// Bare plaintext from before the container existed.
    Legacy,
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
    if start.len() < HEADER_LEN || start[..MAGIC.len()] != MAGIC {
        return Ok(Framing::Legacy);
    }
    let version = start[MAGIC.len()];
    if version != VERSION {
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
    Ok(Framing::Container(codec, dictionary))
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
