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
//!   written to the database carries [`Self::UNASSIGNED`]; storage replaces it
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
    /// Identifies a [`Draft`](crate::Draft).
    DraftId
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
    /// A monotonically increasing per-mailbox modification sequence.
    ///
    /// Used for incremental resynchronization; larger means newer.
    ModSeq,
    u64
);

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
