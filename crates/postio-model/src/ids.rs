//! Newtype identifiers for every entity in the domain model.
//!
//! # Invariants
//!
//! * **Ids are opaque.** Nothing outside the storage layer may construct a
//!   meaningful id, derive an ordering with semantic meaning from one, or parse
//!   structure out of it. They exist to be compared and passed around.
//! * **Ids are local.** A local id is unique within the database that issued it
//!   and is meaningless anywhere else. It is *not* a server identifier — see
//!   [`crate::ServerIdentifiers`] for those.
//! * **Every entity type gets its own id type.** A `MailboxId` can never be
//!   passed where a `MessageId` is expected; the compiler enforces it.
//! * **`0` means "not yet assigned".** A value built in memory but not yet
//!   written to the database carries `Self::UNASSIGNED`; storage replaces it
//!   with the assigned row id on insert. Assigned ids are always `> 0`. Use
//!   `is_assigned()` rather than comparing against a literal.
//! * **Ids serialize transparently** as their inner scalar, so a serialized
//!   entity looks the same as its database row.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! local_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// A local database identifier. See the [module docs](self) for the
        /// invariants that apply to every id type.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            /// The id of an entity that has not been persisted yet.
            pub const UNASSIGNED: Self = Self(0);

            /// Wraps a raw row id. Only the storage layer should call this with
            /// a value it did not itself receive from the database.
            pub const fn new(value: i64) -> Self {
                Self(value)
            }

            /// The raw row id.
            pub const fn get(self) -> i64 {
                self.0
            }

            /// Whether this id refers to a persisted row (`> 0`).
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

local_id!(
    /// Identifies a configured [`Account`](crate::Account).
    AccountId
);
local_id!(
    /// Identifies a sending [`Identity`](crate::Identity).
    IdentityId
);
local_id!(
    /// Identifies a named [`Signature`](crate::Signature).
    SignatureId
);
local_id!(
    /// Identifies a [`Mailbox`](crate::Mailbox).
    MailboxId
);
local_id!(
    /// Identifies a [`Message`](crate::Message).
    ///
    /// This is Postio's own id, *not* the RFC 5322 `Message-ID` header — that is
    /// [`RfcMessageId`].
    MessageId
);
local_id!(
    /// Identifies a [`Thread`](crate::Thread).
    ThreadId
);
local_id!(
    /// Identifies an [`Attachment`](crate::Attachment).
    AttachmentId
);
local_id!(
    /// Identifies a [`Contact`](crate::Contact).
    ContactId
);
local_id!(
    /// Identifies a [`Label`](crate::Label).
    LabelId
);
local_id!(
    /// Identifies a [`ContactGroup`](crate::ContactGroup).
    ContactGroupId
);
local_id!(
    /// Identifies a [`Draft`](crate::Draft).
    DraftId
);
local_id!(
    /// Identifies a cross-account move saga (#188, ADR 0005 Q9) — the
    /// three-phase copy/confirm/remove that stands in for the transaction
    /// two per-account queues cannot share.
    CrossAccountMoveId
);
local_id!(
    /// Identifies a row in the local-first mutation queue.
    ///
    /// Ordering *is* meaningful for this one, and only this one: the queue
    /// drains by ascending id, which is what makes it survive a restart in the
    /// order the user performed the actions.
    OperationId
);

macro_rules! scalar_id {
    ($(#[$doc:meta])* $name:ident, $inner:ty) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// Wraps a raw server-side value.
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The raw server-side value.
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
    /// A server-assigned message number within a mailbox.
    ///
    /// Only meaningful together with the [`UidValidity`] it was observed under:
    /// when the server changes `UIDVALIDITY`, every `Uid` for that mailbox is
    /// stale and the mailbox must be resynchronized from scratch.
    Uid,
    u32
);
scalar_id!(
    /// The generation counter for a mailbox's [`Uid`] space.
    UidValidity,
    u32
);
scalar_id!(
    /// A mailbox's naming generation, as the engine tracks it (#543).
    ///
    /// Opaque above the backend seam: the only operations that mean anything
    /// are equality — "has the server renumbered since I looked?" — and
    /// persistence. For IMAP it carries the `UIDVALIDITY` counter; a backend
    /// whose ids never invalidate reports none at all. Nothing above an
    /// adapter derives, compares magnitudes, or does arithmetic on one.
    Generation,
    u32
);
scalar_id!(
    /// A monotonically increasing per-mailbox modification sequence.
    ///
    /// Used for incremental resynchronization; larger means newer.
    ModSeq,
    u64
);

/// The server's own name for a message, whatever the protocol (#543,
/// ADR 0018 Q2).
///
/// Opaque above the backend seam: a JMAP `Email` id or a Gmail message id
/// is a server-wide string with no structure worth knowing. The IMAP
/// adapter packs its generation-and-uid pair into one and derives its wire
/// `Uid` back out; nothing outside that adapter reads structure into a
/// `RemoteId`, and nothing outside a backend adapter constructs one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteId(String);

impl RemoteId {
    /// Wraps a server-assigned message identity.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The identity, as the text a database column or wire call carries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RemoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The key of a blob in the content-addressed blob store.
///
/// Opaque to the domain model: it is produced and resolved by the storage layer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(String);

impl BlobId {
    /// Wraps a blob store key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The blob store key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An RFC 5322 `Message-ID`, as it appears in `Message-ID`, `In-Reply-To` and
/// `References` headers.
///
/// # Invariants
///
/// * The stored value is trimmed and **always** wrapped in angle brackets, so
///   `a@b` and `<a@b>` are the same value. Construction normalizes; there is no
///   way to build an unnormalized one, including through `Deserialize`.
/// * Original case is preserved for round-tripping, but equality, ordering and
///   hashing are **case-insensitive** — this is what makes JWZ threading match
///   messages whose headers were rewritten in transit.
/// * This is a *header* value and is not unique in any local database. The local
///   identity of a message is [`MessageId`].
#[derive(Debug, Clone)]
pub struct RfcMessageId(String);

impl RfcMessageId {
    /// Normalizes and wraps a `Message-ID` header value.
    pub fn new(value: impl AsRef<str>) -> Self {
        let trimmed = value.as_ref().trim();
        let inner = trimmed.trim_start_matches('<').trim_end_matches('>').trim();
        Self(format!("<{inner}>"))
    }

    /// The normalized value, angle brackets included.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The value without its angle brackets.
    pub fn without_brackets(&self) -> &str {
        self.0.trim_start_matches('<').trim_end_matches('>')
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
        // Goes through `new` so a deserialized value obeys the same invariants.
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `RfcMessageId`'s `PartialEq` is case-insensitive by design (see the
    // type's own doc comment: it is what lets JWZ threading match a header
    // rewritten in transit), which means `Ord` and `Hash` are *hand-written*
    // rather than derived -- `derive`d versions would compare the original,
    // case-preserved bytes and silently disagree with `Eq` about which
    // values are the same one. That disagreement is exactly the kind of bug
    // that breaks a `HashSet` without ever panicking: a lookup that should
    // hit misses, and two "equal" values both end up stored.
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
        // The point above is that case does not matter, not that nothing
        // does -- a hand-written `Ord` that always returned `Equal` would
        // pass both tests above and be useless for anything sorted.
        let a = RfcMessageId::new("<a@example.com>");
        let b = RfcMessageId::new("<b@example.com>");

        assert_eq!(a.cmp(&b), std::cmp::Ordering::Less);
        assert_eq!(b.cmp(&a), std::cmp::Ordering::Greater);
    }
}
