//! Postio's domain model: the types every other crate speaks.
//!
//! This crate is deliberately *pure*. It contains no SQL, no IMAP, no GTK and no
//! I/O of any kind — only value types and the logic that belongs to them
//! (flag canonicalization, special-use resolution, subject normalization).
//! `postio-storage` persists these types, `postio-imap` translates the wire into
//! them, `postio-gtk` renders them, and none of that leaks back here. That is
//! what lets a second protocol or a second frontend be added without reshaping
//! the model, and CI enforces it.
//!
//! # Shape of the model
//!
//! * [`Account`] owns one or more [`Identity`] values and a set of [`Mailbox`]
//!   folders, each with a [`MailboxRole`].
//! * A [`Mailbox`] holds [`Message`] values, which carry [`FlagSet`],
//!   [`Label`] references, [`Attachment`] metadata, [`Headers`],
//!   [`ServerIdentifiers`] and [`LocalSyncState`].
//! * Messages are grouped into a [`Thread`] by [`threading::assign`], which
//!   reads [`Message::reference_chain`] and [`normalize_subject`] and places
//!   one message at a time rather than rethreading a mailbox.
//! * [`Contact`] accumulates addresses that have been seen; [`Draft`] is a
//!   message being composed.
//!
//! # Identifier invariants
//!
//! Every entity has its own newtype id, `0` means "not yet persisted", and
//! server-assigned identifiers are kept separate from local ones. See the
//! [`ids`] module for the full set of rules.
//!
//! # Test corpus
//!
//! Behind the off-by-default `test-corpus` feature this crate also ships the
//! `.eml` fixture corpus and its loader ([`test_corpus`]), so that every crate
//! in the workspace can test against realistic mail without touching the
//! network. It is dev-dependency-only; nothing in a normal build pulls it in.
//!
//! # Flag invariants
//!
//! Flags have exactly one canonical representation, keywords compare
//! case-insensitively, and `\Recent` is never persisted. See [`Flag`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod account;
pub mod address;
pub mod attachment;
pub mod contact;
pub mod draft;
pub mod flag;
pub mod headers;
pub mod ids;
pub mod label;
pub mod mailbox;
pub mod message;
pub mod mime;
pub mod subject;
#[cfg(feature = "test-corpus")]
pub mod test_corpus;
pub mod thread;
pub mod threading;

pub use account::{Account, AuthMethod, Identity, ServerConfig, Signature, TransportSecurity};
pub use address::EmailAddress;
pub use attachment::{Attachment, Disposition};
pub use contact::Contact;
pub use draft::{Draft, DraftKind, DraftState};
pub use flag::{Flag, FlagSet};
pub use headers::{Header, Headers};
pub use ids::{
    AccountId, AttachmentId, BlobId, ContactId, DraftId, IdentityId, LabelId, MailboxId, MessageId,
    ModSeq, RfcMessageId, ThreadId, Uid, UidValidity,
};
pub use label::Label;
pub use mailbox::{Mailbox, MailboxCounts, MailboxRole};
pub use message::{BodyState, LocalSyncState, Message, MessageBody, ServerIdentifiers};
pub use mime::{ParsedMessage, ParsedPart};
pub use subject::{is_reply, normalize_subject};
pub use thread::Thread;
pub use threading::{Assignment, ThreadCue, ThreadIndex, assign, claimed_ids};
