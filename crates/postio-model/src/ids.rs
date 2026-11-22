//! Ids are opaque and local: only storage constructs meaningful ones, and a local id means nothing
//! outside its
//! database (server ids are `ServerIdentifiers`). `0` is `UNASSIGNED`; assigned ids are always `>
//! 0`, so use `is_assigned()`.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! local_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub const UNASSIGNED: Self = Self(0);

            /// Wraps a raw row id. Only the storage layer should call this with
            /// a value it did not itself receive from the database.
            pub const fn new(value: i64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> i64 {
                self.0
            }

            pub const fn is_assigned(self) -> bool {
                self.0 > 0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::UNASSIGNED
            }
        }

        impl From<i64> for $name {
            fn from(value: i64) -> Self {
                Self(value)
            }
        }

        impl From<$name> for i64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

local_id!(AccountId);
local_id!(IdentityId);
local_id!(SignatureId);
local_id!(MailboxId);
local_id!(
    /// Postio's own id, not the RFC 5322 `Message-ID` (`RfcMessageId`).
    MessageId
);
local_id!(ThreadId);
local_id!(AttachmentId);
local_id!(ContactId);
local_id!(LabelId);
local_id!(
    /// A row of `addresses`: one address however many messages name it, so search facets count a
    /// person once.
    AddressId
);
local_id!(ContactGroupId);
local_id!(DraftId);
local_id!(
    /// A cross-account move saga: the three-phase copy/confirm/remove standing in for a transaction
    /// two queues cannot share (#188, ADR 0005 Q9).
    CrossAccountMoveId
);
local_id!(
    /// Ordering is meaningful for this id only: the queue drains by ascending id, so it survives a
    /// restart in the order the user acted.
    OperationId
);
local_id!(UnsubscribeActivationId);
local_id!(DeliveryId);
local_id!(ReminderId);

macro_rules! scalar_id {
    ($(#[$doc:meta])* $name:ident, $inner:ty) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

scalar_id!(
    /// Only meaningful with the `UidValidity` it was observed under: when that changes, every `Uid`
    /// for the mailbox is stale.
    Uid,
    u32
);
scalar_id!(UidValidity, u32);
scalar_id!(
    /// Opaque above the backend seam: only equality (has the server renumbered?) and persistence
    /// mean anything.
    /// For IMAP it carries `UIDVALIDITY`; nothing above an adapter does arithmetic on it (#543).
    Generation,
    u32
);
scalar_id!(
    /// Larger means newer; used for incremental resynchronization.
    ModSeq,
    u64
);

/// The server's own name for a message (#543, ADR 0018 Q2). Opaque above the backend seam: the IMAP
/// adapter packs
/// generation and uid into one; nothing else reads structure into it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteId(String);

impl RemoteId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(String);

impl BlobId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stored trimmed and always in angle brackets, so `a@b` and `<a@b>` are equal; construction,
/// including `Deserialize`,
/// normalizes. Equality, ordering and hashing are case-insensitive so JWZ threading matches headers
/// rewritten in
/// transit; the case is kept. Not unique locally: the local identity is `MessageId`.
#[derive(Debug, Clone)]
pub struct RfcMessageId(String);

impl RfcMessageId {
    pub fn new(value: impl AsRef<str>) -> Self {
        let trimmed = value.as_ref().trim();
        let inner = trimmed.trim_start_matches('<').trim_end_matches('>').trim();
        Self(format!("<{inner}>"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn without_brackets(&self) -> &str {
        self.0.trim_start_matches('<').trim_end_matches('>')
    }

    /// Lowercased lookup key for `thread_links` (#1587): `COLLATE NOCASE` forced a collated index,
    /// which Turso's planner
    /// will not bind an equality through. Only the key: `as_str` keeps the case, because an emitted
    /// `References` must
    /// quote the parent byte-for-byte.
    pub fn folded(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl PartialEq for RfcMessageId {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for RfcMessageId {}

impl PartialOrd for RfcMessageId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RfcMessageId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .to_ascii_lowercase()
            .cmp(&other.0.to_ascii_lowercase())
    }
}

impl std::hash::Hash for RfcMessageId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_ascii_lowercase().hash(state);
    }
}

impl fmt::Display for RfcMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for RfcMessageId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RfcMessageId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `Ord` and `Hash` are hand-written: derived ones compare case-preserved bytes and would
    // disagree with the
    // case-insensitive `Eq`, silently breaking a `HashSet`.
    #[test]
    fn case_variants_of_the_same_id_order_as_equal() {
        let lower = RfcMessageId::new("<abc123@example.com>");
        let upper = RfcMessageId::new("<ABC123@EXAMPLE.COM>");

        assert_eq!(lower.cmp(&upper), std::cmp::Ordering::Equal);
        assert_eq!(lower.partial_cmp(&upper), Some(std::cmp::Ordering::Equal));
    }

    #[test]
    fn case_variants_of_the_same_id_hash_to_the_same_bucket() {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        seen.insert(RfcMessageId::new("<abc123@example.com>"));

        assert!(
            !seen.insert(RfcMessageId::new("<ABC123@EXAMPLE.COM>")),
            "a case-differing id must land on the same hash bucket and be \
             recognised as already present, or Eq and Hash disagree"
        );
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn ordering_still_distinguishes_genuinely_different_ids() {
        let a = RfcMessageId::new("<a@example.com>");
        let b = RfcMessageId::new("<b@example.com>");

        assert_eq!(a.cmp(&b), std::cmp::Ordering::Less);
        assert_eq!(b.cmp(&a), std::cmp::Ordering::Greater);
    }
}
