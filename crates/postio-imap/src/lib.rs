//! IMAP backend for Postio.
//!
//! Two pieces are in place so far, both prerequisites for opening a
//! connection at all:
//!
//! * [`secret`] — where an account's password lives. The OS keyring by
//!   default, a user-supplied command as the escape hatch, and plaintext
//!   never.
//! * [`discovery`] — the first-run autoconfig probe that turns an email
//!   address into server settings.
//!
//! Development in this repository is test-first: write the failing test,
//! then the implementation. See `CLAUDE.md`.

#![warn(missing_docs)]

pub mod discovery;
pub mod secret;
