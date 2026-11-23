//! `backend` is the protocol-free seam and mock; `imap` is the `io-imap` implementation. Features:
//! `imap` (default), `test-server` (in-process IMAP server on loopback), `test-corpus`.
//! Live-server tests are `#[ignore]`d and read `POSTIO_TEST_IMAP_USER`/`POSTIO_TEST_IMAP_PASSWORD`
//! (and `POSTIO_TEST_IMAP_HOST` when no preset covers the domain); they must only read, or clean up
//! what they change.

pub mod auth;
pub mod backend;
pub mod cancel;
pub mod discovery;
#[cfg(feature = "imap")]
pub mod imap;

/// Optional so a build that never opens one does not pull in what it needs (#1278).
#[cfg(feature = "maildir")]
pub mod maildir;
pub mod oauth;
pub mod secret;
mod single_flight;
#[cfg(feature = "test-server")]
pub mod test_server;
