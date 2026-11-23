//! The store's master key (ADR 0014 Q3) and its three derived subkeys: database, blob contents,
//! blob ids.
//! One keyring entry, no plaintext fallback: a locked keyring means the mail does not open.
//! The context strings in [`Purpose::context`] are on-disk format; changing one stops every store
//! opening. Fetching the key is `postio-session`'s.

use zeroize::{Zeroize, Zeroizing};

/// Part of the on-disk format like [`Purpose::context`]: change it and every installation looks
/// like a first run and mints a second key.
/// No `@`, so it cannot collide with an account's entry; the label reads "Postio (local store
/// encryption key)" in `seahorse`.
pub const STORE_KEY_ENTRY: &str = "local store encryption key";

pub const KEY_BYTES: usize = 32;

/// Not `#[non_exhaustive]`: a fourth purpose is a schema decision, and the compiler pointing at
/// every `match` is the review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Database,
    BlobContent,
    /// Keys the BLAKE3 blob id, so dedup survives inside a store while a directory listing cannot
    /// confirm a known file.
    BlobId,
}

impl Purpose {
    pub const fn context(self) -> &'static str {
        match self {
            Self::Database => "postio db",
            Self::BlobContent => "postio blob content",
            Self::BlobId => "postio blob id",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// Refused, not padded or truncated: a quietly different key opens nothing and looks like data
    /// loss.
    #[error(
        "the stored store key is not {expected} hexadecimal characters (it is {found}); \
         the keyring entry is corrupt or was written by something else"
    )]
    Malformed { expected: usize, found: usize },
    #[error("the stored store key contains something that is not a hexadecimal digit")]
    NotHex,
}

/// Zeroized on drop and never rendered; call [`expose`](Self::expose) or [`to_hex`](Self::to_hex)
/// only where the bytes are needed.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreKey(Zeroizing<[u8; KEY_BYTES]>);

impl StoreKey {
    /// From `getrandom`: the kernel's pool, not anything this process could be persuaded to
    /// reproduce.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
        // No entropy source is fatal: an invented key is a key an attacker can invent.
        getrandom::fill(bytes.as_mut()).expect("the operating system has no random number source");
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        if text.len() != KEY_BYTES * 2 {
            return Err(KeyError::Malformed {
                expected: KEY_BYTES * 2,
                found: text.len(),
            });
        }
        let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
        // The length is checked above, so `as_chunks` leaves no remainder.
        let (pairs, _) = text.as_bytes().as_chunks::<2>();
        for (slot, pair) in bytes.iter_mut().zip(pairs) {
            let digits = std::str::from_utf8(pair).map_err(|_| KeyError::NotHex)?;
            *slot = u8::from_str_radix(digits, 16).map_err(|_| KeyError::NotHex)?;
        }
        Ok(Self(bytes))
    }

    /// Hex, not base64: no padding, alphabet variants or case question, and it cannot be mistaken
    /// for a typed password.
    pub fn to_hex(&self) -> Zeroizing<String> {
        to_hex(&self.0)
    }

    pub fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    /// Deterministic, so a store written yesterday reads today.
    pub fn derive(&self, purpose: Purpose) -> Subkey {
        Subkey(Zeroizing::new(blake3::derive_key(
            purpose.context(),
            &*self.0,
        )))
    }
}

impl std::fmt::Debug for StoreKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not `finish_non_exhaustive`: it invites adding a field, and nothing here may be printed.
        formatter.write_str("StoreKey(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Subkey(Zeroizing<[u8; KEY_BYTES]>);

impl Subkey {
    pub fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    pub fn to_hex(&self) -> Zeroizing<String> {
        to_hex(&self.0)
    }
}

impl std::fmt::Debug for Subkey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Subkey(<redacted>)")
    }
}

/// Written out: a `hex` crate would return a plain `String` that lingers in freed memory.
fn to_hex(bytes: &[u8; KEY_BYTES]) -> Zeroizing<String> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = Zeroizing::new(String::with_capacity(KEY_BYTES * 2));
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// `Zeroizing` covers drop; this covers `*key = other`, which drops the old value after the
/// assignment.
impl Zeroize for StoreKey {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Zeroize for Subkey {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

/// The blob store's two subkeys travel together, which also keeps the master key out of `blob.rs`.
/// Cloneable because [`crate::BlobStore`] is.
#[derive(Clone, PartialEq, Eq)]
pub struct BlobKeys {
    content: Subkey,
    id: Subkey,
}

impl BlobKeys {
    pub fn derive(master: &StoreKey) -> Self {
        Self {
            content: master.derive(Purpose::BlobContent),
            id: master.derive(Purpose::BlobId),
        }
    }

    pub fn content(&self) -> &Subkey {
        &self.content
    }

    pub fn id(&self) -> &Subkey {
        &self.id
    }
}

impl std::fmt::Debug for BlobKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BlobKeys(<redacted>)")
    }
}

impl Zeroize for BlobKeys {
    fn zeroize(&mut self) {
        self.content.zeroize();
        self.id.zeroize();
    }
}
