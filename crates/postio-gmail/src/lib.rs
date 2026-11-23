//! Gmail REST [`MailBackend`] adapter over `io-gmail` (ADR 0018). Labels, not folders:
//! system labels are the mailboxes, Archive is the messages no system label claims,
//! and `\Seen` is the absence of `UNREAD`. Identity is the Gmail id; the uid is a synthetic
//! position.
//!
//! [`MailBackend`]: postio_account::backend::MailBackend

pub mod backend;
mod connection;
mod convert;

pub use backend::GmailBackend;
