//! JMAP [`MailBackend`] adapter over `io-jmap` (ADR 0018). The
//! [`RemoteId`](postio_model::RemoteId) is the server's `Email` id verbatim; the `uid` is a
//! synthetic position.
//! No CondStore claim, no sectioned body fetches, no `find_by_message_id` (io-jmap's `Email/query`
//! has no header filter).
//!
//! [`MailBackend`]: postio_account::backend::MailBackend

pub mod backend;
mod convert;
mod error;
mod session;

pub use backend::JmapBackend;
pub use session::JmapConnection;
