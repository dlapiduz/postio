//! The blob container: a fixed header (magic, version, codec, dictionary id, cipher) and, in
//! version 2, a nonce prefix.
//! Version 1 (compressed, unencrypted) is read-only; a file without [`MAGIC`] is legacy plaintext,
//! read verbatim and never migrated.
//! [`MAGIC`] begins with NUL, which no raw message, text or HTML body can.

use crate::blob::seal;
use crate::error::{Error, Result};

pub(crate) const MAGIC: [u8; 5] = [0x00, b'P', b'B', b'L', b'B'];

pub(crate) const VERSION_PLAIN: u8 = 1;

pub(crate) const VERSION: u8 = 2;

/// Header bytes shared by both versions: `MAGIC` (5), version (1), codec (1), dictionary id (4,
/// little-endian), cipher (1).
pub(crate) const FIXED_LEN: usize = MAGIC.len() + 7;

/// [`FIXED_LEN`] plus the nonce prefix; a version 1 payload begins at `FIXED_LEN`.
pub(crate) const HEADER_LEN: usize = FIXED_LEN + seal::NONCE_PREFIX_LEN;

const CIPHER_NONE: u8 = 0;

const CIPHER_XCHACHA20_POLY1305: u8 = 1;

/// Always 0: text moved into `messages` rows (ADR 0020) and payloads are largely compressed
/// already.
/// The field stays because removing it would mean rewriting every blob.
pub(crate) const NO_DICTIONARY: u32 = 0;

/// Lower than [`LEVEL`]: the probe runs on every write, so a cheap pessimistic answer beats an
/// exact slow one.
const PROBE_LEVEL: i32 = 1;

/// zstd's default: most of the ratio for text, and decompression speed barely varies with level.
pub(crate) const LEVEL: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    /// Already-compressed payloads (JPEG, PNG, PDF, ZIP): zstd would cost CPU and grow them.
    None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framing {
    Container(Container),
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Container {
    pub(crate) codec: Codec,
    /// The per-blob nonce prefix, or `None` for a version 1 container.
    pub(crate) nonce: Option<[u8; seal::NONCE_PREFIX_LEN]>,
}

impl Container {
    pub(crate) fn payload_at(self) -> usize {
        if self.nonce.is_some() {
            HEADER_LEN
        } else {
            FIXED_LEN
        }
    }
}

/// `start` may be short; a blob smaller than a header is legacy.
///
/// # Errors
///
/// [`Error::UnreadableBlob`] for a container naming a version or codec this build does not know (a
/// newer Postio): guessing would return wrong bytes under a digest that promises otherwise.
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

/// Version 1 has no cipher and no nonce, version 2 always has both; any other combination is
/// refused, not guessed.
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

/// Decided from the first chunk: a sample zstd cannot get below 90% is taken as already compressed.
pub(crate) fn looks_compressible(sample: &[u8]) -> bool {
    // Too small to tell or matter; compressing keeps small blobs on one path.
    if sample.len() < 512 {
        return true;
    }
    let Ok(trial) = zstd::bulk::compress(sample, PROBE_LEVEL) else {
        return false;
    };
    trial.len() * 10 < sample.len() * 9
}
