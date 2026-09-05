//! The store's master key, and the subkeys derived from it.
//!
//! ADR 0014 Q3. One 32-byte key per store, generated from the OS RNG on first
//! open and kept in the Secret Service keyring beside the account credentials
//! — the same seam, the same locked-keyring behaviour, and the same
//! **no-plaintext-fallback** rule `postio_account::secret` already enforces for
//! passwords. A locked keyring means the mail does not open; there is no
//! "open it read-only anyway".
//!
//! # Why one key and three subkeys
//!
//! The database, the blob contents and the blob *ids* are three different
//! uses, and a weakness in one must not reach the others. Three keyring
//! entries would separate them and would also mean three things to lose, three
//! things to migrate, and three chances for a store to end up half-keyed.
//! BLAKE3's `derive_key` separates them from one entry instead, which is what
//! it is for.
//!
//! # The contexts are part of the on-disk format
//!
//! Change one of the strings in [`Purpose::context`] and every existing
//! store's database subkey changes with it — which is to say every existing
//! store stops opening. They are pinned by a test for that reason, not for
//! tidiness.
//!
//! # What lives here and what does not
//!
//! This module is the *material*. Fetching it — asking the keyring, minting
//! one on first run, routing a locked keyring to the surface that asks the
//! user to unlock it — is `postio-session`'s, because that is where the
//! `SecretStore` seam lives and this crate must not depend on it.

use zeroize::{Zeroize, Zeroizing};

/// The keyring entry the master key is kept under.
///
/// Here rather than in `postio-session` — which owns the `SecretStore` seam
/// and does the actual fetching — because the *name* is part of the on-disk
/// format in the same way [`Purpose::context`] is: change it and every
/// existing installation looks like a first run, mints a second key, and
/// leaves the store it can no longer open sitting on disk. It has more than
/// one reader now, and a magic string with two copies has a way of acquiring
/// a third that differs.
///
/// Not an address, and it cannot become one: there is no `@` in it, so it can
/// never collide with an account's own entry however many accounts an
/// installation grows. Written out rather than derived from the store path
/// because the key has to be findable by a person in `seahorse` when they
/// want to know what Postio keeps — the label reads
/// "Postio (local store encryption key)".
pub const STORE_KEY_ENTRY: &str = "local store encryption key";

/// How long a key is, in bytes. Both the master key and every subkey.
pub const KEY_BYTES: usize = 32;

/// What a subkey is for.
///
/// The three uses ADR 0014 separates. Not `#[non_exhaustive]`: a fourth
/// purpose is a schema decision, and the compiler pointing at every `match`
/// is the review that decision deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// SQLCipher's `PRAGMA key` for the database.
    Database,
    /// The AEAD key each blob's contents are encrypted under.
    BlobContent,
    /// The key a blob's id is BLAKE3-*keyed* with, so that dedup survives
    /// inside a store while a directory listing no longer confirms whether
    /// this mailbox holds a known file.
    BlobId,
}

impl Purpose {
    /// The BLAKE3 `derive_key` context for this purpose.
    ///
    /// **These strings are format.** See the module docs.
    pub const fn context(self) -> &'static str {
        match self {
            Self::Database => "postio db",
            Self::BlobContent => "postio blob content",
            Self::BlobId => "postio blob id",
        }
    }
}

/// Everything that can go wrong turning stored text back into a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The stored text is not `KEY_BYTES * 2` hex characters.
    ///
    /// Refused rather than padded or truncated. A key that is quietly
    /// *different* opens nothing and looks like data loss; a key that will
    /// not parse says so.
    #[error(
        "the stored store key is not {expected} hexadecimal characters (it is {found}); \
         the keyring entry is corrupt or was written by something else"
    )]
    Malformed {
        /// How many characters a key is.
        expected: usize,
        /// How many were found.
        found: usize,
    },
    /// A character outside `0-9a-fA-F`.
    #[error("the stored store key contains something that is not a hexadecimal digit")]
    NotHex,
}

/// A store's 32-byte master key.
///
/// Zeroized on drop, and never rendered: [`Debug`] says only that it exists.
/// Reach for [`expose`](Self::expose) or [`to_hex`](Self::to_hex) at the
/// moment the bytes are actually needed, so every use is short and obvious in
/// review — the discipline `postio_account::secret::Password` already keeps.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreKey(Zeroizing<[u8; KEY_BYTES]>);

impl StoreKey {
    /// A new key from the operating system's RNG.
    ///
    /// `getrandom` rather than a seeded generator: this is the one secret
    /// that opens everything, and it must come from the kernel's pool rather
    /// than from anything this process could be persuaded to reproduce.
    pub fn generate() -> Self {
        let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
        // A failure here means the kernel has no entropy source, which is
        // not a state a mail client can carry on in — a key it invented
        // would be a key an attacker can invent too.
        getrandom::fill(bytes.as_mut()).expect("the operating system has no random number source");
        Self(bytes)
    }

    /// A key from bytes somebody else already has.
    pub fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Parses the form [`to_hex`](Self::to_hex) writes.
    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        if text.len() != KEY_BYTES * 2 {
            return Err(KeyError::Malformed {
                expected: KEY_BYTES * 2,
                found: text.len(),
            });
        }
        let mut bytes = Zeroizing::new([0u8; KEY_BYTES]);
        // The length is checked above, so every byte has its pair and the
        // remainder `as_chunks` reports is empty.
        let (pairs, _) = text.as_bytes().as_chunks::<2>();
        for (slot, pair) in bytes.iter_mut().zip(pairs) {
            let digits = std::str::from_utf8(pair).map_err(|_| KeyError::NotHex)?;
            *slot = u8::from_str_radix(digits, 16).map_err(|_| KeyError::NotHex)?;
        }
        Ok(Self(bytes))
    }

    /// The key as lowercase hex — how it is kept in the keyring, which stores
    /// text.
    ///
    /// Hex rather than base64: no padding, no alphabet variants, no case
    /// question, and a keyring browser showing it cannot mistake it for a
    /// password somebody typed.
    pub fn to_hex(&self) -> Zeroizing<String> {
        to_hex(&self.0)
    }

    /// The bytes. Short-lived call sites only; see the type's own docs.
    pub fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    /// The subkey for one purpose.
    ///
    /// Deterministic: the same master key and the same purpose give the same
    /// subkey on every open, which is what lets a store that was written
    /// yesterday be read today.
    pub fn derive(&self, purpose: Purpose) -> Subkey {
        Subkey(Zeroizing::new(blake3::derive_key(
            purpose.context(),
            &*self.0,
        )))
    }
}

impl std::fmt::Debug for StoreKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not `finish_non_exhaustive`, which would invite somebody to add a
        // field later. There is nothing here that may ever be printed.
        formatter.write_str("StoreKey(<redacted>)")
    }
}

/// A key derived from a [`StoreKey`] for one [`Purpose`].
///
/// Same discipline as the master key: zeroized, never rendered.
#[derive(Clone, PartialEq, Eq)]
pub struct Subkey(Zeroizing<[u8; KEY_BYTES]>);

impl Subkey {
    /// The bytes. Short-lived call sites only.
    pub fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }

    /// The subkey as lowercase hex — what SQLCipher's `PRAGMA key = x'…'`
    /// wants, and what a test asserts against.
    pub fn to_hex(&self) -> Zeroizing<String> {
        to_hex(&self.0)
    }
}

impl std::fmt::Debug for Subkey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Subkey(<redacted>)")
    }
}

/// Lowercase hex, into a buffer that overwrites itself when it drops.
///
/// Written out rather than reached for: a `hex` crate would be a dependency
/// for eight lines, and it would hand back a plain `String` that lingers in
/// freed memory — which is the whole thing this is trying not to do.
fn to_hex(bytes: &[u8; KEY_BYTES]) -> Zeroizing<String> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = Zeroizing::new(String::with_capacity(KEY_BYTES * 2));
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Overwrite a key that is being replaced rather than dropped.
///
/// `Zeroizing` covers the drop; this covers `*key = other`, which drops the
/// old value *after* the assignment and so is not the same thing.
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

/// The two subkeys the blob store works under.
///
/// [`Purpose::BlobContent`] and [`Purpose::BlobId`] are always wanted together
/// — a store that could encrypt but not name, or name but not encrypt, is not
/// a state worth being able to express — so they travel as one value. It also
/// keeps the *master* key out of `blob.rs`: the only thing that ever holds all
/// of it is the composition root that read it from the keyring.
///
/// Cloneable, because [`crate::BlobStore`] is: a blob store is a path and its
/// keys and nothing else, and half the tree clones one.
#[derive(Clone, PartialEq, Eq)]
pub struct BlobKeys {
    content: Subkey,
    id: Subkey,
}

impl BlobKeys {
    /// Derives both subkeys from the store's master key.
    pub fn derive(master: &StoreKey) -> Self {
        Self {
            content: master.derive(Purpose::BlobContent),
            id: master.derive(Purpose::BlobId),
        }
    }

    /// The key a blob's contents are encrypted under.
    pub fn content(&self) -> &Subkey {
        &self.content
    }

    /// The key a blob's id is BLAKE3-keyed with.
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
