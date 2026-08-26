//! IMAP backend for Postio.
//!
//! The crate has two halves. [`backend`] is the seam: the `MailBackend`
//! trait, the domain types it speaks, and an in-memory mock. Nothing in it
//! knows what IMAP is, and nothing above it needs to. [`imap`] is one
//! implementation of that seam, built on the pre-1.0 `io-imap` crate, and is
//! where every protocol type is confined. ADR 0001 explains why that division
//! is load-bearing rather than tidy.
//!
//! Around them:
//!
//! * [`secret`] — where an account's password lives. The OS keyring by
//!   default, a user-supplied command as the escape hatch, and plaintext
//!   never.
//! * [`discovery`] — the first-run autoconfig probe that turns an email
//!   address into server settings.
//! * [`cancel`] — the "stop what you are doing" token every operation that
//!   reaches a server is raced against.
//!
//! # Features
//!
//! `imap` (default) compiles the `io-imap`-backed implementation. With it off,
//! the crate is the seam, the mock, discovery and the keyring — which is all
//! `postio-sync` needs, and keeps the protocol crate out of its graph.
//!
//! `test-server` (off) compiles `test_server`, an in-process IMAP server on
//! a loopback port that the real client stack can be pointed at. It is what
//! puts `io-imap` itself under test — the mock sits above the protocol and a
//! scripted transcript cannot answer a sequence nobody wrote down — and it is
//! where the lies a real provider tells are reproduced on demand. `test-corpus`
//! (off) hands the `.eml` fixtures to both it and the mock.
//!
//! # Live tests
//!
//! `cargo test -p postio-imap` never touches the network: every live-server
//! test is `#[ignore]`d, scattered next to the mock-backed tests for the same
//! operation (`imap_session.rs` for connect and the post-auth capability
//! re-read, `imap_mailboxes.rs` for folder discovery, `imap_fetch.rs` for
//! header fetch, `imap_body.rs` for body fetch). None of them name a
//! provider: they read `POSTIO_TEST_IMAP_USER` and `POSTIO_TEST_IMAP_PASSWORD`
//! (an app-specific password where the account needs one) from the
//! environment, resolve server settings from [`discovery`]'s preset table
//! when the address's domain has a row there, and otherwise fall back to
//! `POSTIO_TEST_IMAP_HOST`. Run them against any real IMAP account with:
//!
//! ```text
//! POSTIO_TEST_IMAP_USER=you@example.com \
//! POSTIO_TEST_IMAP_PASSWORD=your-app-specific-password \
//! cargo test -p postio-imap -- --ignored
//! ```
//!
//! Every live test today only reads (`LIST`, `SELECT`, `FETCH` with
//! `BODY.PEEK`) and so leaves the account exactly as it found it. A future
//! live test for a mutating operation (flag store, move, append) must clean
//! up whatever it creates or changes as part of the same test.
//!
//! Development in this repository is test-first: write the failing test,
//! then the implementation. See `CLAUDE.md`.

pub mod auth;
pub mod backend;
pub mod cancel;
pub mod discovery;
#[cfg(feature = "imap")]
pub mod imap;
pub mod oauth;
pub mod secret;
#[cfg(feature = "test-server")]
pub mod test_server;
