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
//! Development in this repository is test-first: write the failing test,
//! then the implementation. See `CLAUDE.md`.

#![warn(missing_docs)]

pub mod backend;
pub mod cancel;
pub mod discovery;
#[cfg(feature = "imap")]
pub mod imap;
pub mod secret;
