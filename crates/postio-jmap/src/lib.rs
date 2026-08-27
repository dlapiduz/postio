//! JMAP [`MailBackend`] adapter over Pimalaya's `io-jmap` (#544, ADR 0018).
//!
//! The second implementation of the seam ADR 0001 drew: everything above
//! `postio-imap`'s `backend` module keeps its one code path, and this crate
//! answers it in RFC 8620/8621 — Fastmail's native protocol — instead of
//! IMAP.
//!
//! # Identity
//!
//! A JMAP `Email` id is a server-wide opaque string with no generations to
//! invalidate, so the [`RemoteId`](postio_model::RemoteId) this adapter
//! mints is the id **verbatim** — nothing packed, nothing derived. The
//! `uid` on a fetched message is a synthetic enumeration position (see
//! [`backend`]); identity, not the uid, is what names a message (#543).
//!
//! # What this first slice does not do
//!
//! * **No CondStore claim**: the engine's incremental pull speaks
//!   per-message `MODSEQ`, which JMAP does not have. Until the native delta
//!   seam (ADR 0018 Q3), a resync is a full re-enumeration — correct, and
//!   cheap enough over `Email/query`'s windowing.
//! * **No sectioned body fetches**: `fetch_part` serves
//!   [`BodyPart::Whole`](postio_imap::backend::BodyPart) from the raw-blob
//!   download. Fetched headers carry no `BODYSTRUCTURE`, so the backfill
//!   takes its documented no-sections path and fetches the whole message.
//! * **No `find_by_message_id`**: io-jmap 0.3's `Email/query` filter has no
//!   header condition; the trait default ("this backend cannot search")
//!   stands, which the cross-account saga reads as *unconfirmed*.
//!
//! [`MailBackend`]: postio_imap::backend::MailBackend

pub mod backend;
mod convert;
mod error;
mod session;

pub use backend::JmapBackend;
pub use session::JmapConnection;
