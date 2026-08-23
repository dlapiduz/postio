//! SMTP sending for Postio, built on the pre-1.0 `io-smtp` crate.
//!
//! A sibling of `postio-imap` rather than a dependent of it — see
//! CLAUDE.md's architecture diagram, where both hang off `postio-sync` as
//! independent protocol crates. Anything shaped like both protocols need
//! (the account password, a cancellation token) is duplicated in miniature
//! rather than shared, which is what keeps sending mail from ever requiring
//! the IMAP stack to compile.
//!
//! # Layout
//!
//! * [`session`] — [`session::SmtpSession`]: connect, authenticate, send one
//!   message, quit. No pool, unlike IMAP: a send opens a connection, runs
//!   one mail transaction, and closes it.
//! * [`error`] — [`error::SmtpError`], with the same "branch on a predicate,
//!   never the variant" contract `postio_imap::backend::BackendError` uses.
//! * [`settings`] — [`settings::ConnectionSettings`]: host, port, transport
//!   security, matching `postio_model::ServerConfig`.
//! * [`transport`] — sockets and TLS, and [`transport::ScriptedConnector`]
//!   for testing the whole handshake with no socket at all.
//! * [`cancel`] — the "stop what you are doing" token a send is raced
//!   against.
//!
//! Development in this repository is test-first: write the failing test,
//! then the implementation. See `CLAUDE.md`.

pub mod cancel;
pub mod error;
pub mod session;
pub mod settings;
pub mod transport;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {}
}
