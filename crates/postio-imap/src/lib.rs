//! IMAP backend for Postio.
//!
//! Three pieces are in place so far, all prerequisites for opening a
//! connection at all:
//!
//! * [`secret`] — where an account's password lives. The OS keyring by
//!   default, a user-supplied command as the escape hatch, and plaintext
//!   never.
//! * [`discovery`] — the first-run autoconfig probe that turns an email
//!   address into server settings.
//! * [`cancel`] — the "stop what you are doing" token every operation that
//!   reaches a server is raced against.
//!
//! Development in this repository is test-first: write the failing test,
//! then the implementation. See `CLAUDE.md`.

#![warn(missing_docs)]

pub mod backend;
pub mod cancel;
pub mod discovery;
pub mod secret;
