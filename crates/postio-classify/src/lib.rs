//! Classification for Postio Focus (`specs/007-postio-focus`, research R8).
//!
//! One fixed-schema answer per message: whether to file it away as spam or an
//! update, whether to hold it for a digest, and what it asks of the user. It
//! is decided in layers, and the guards come first. This crate cannot send
//! mail. Nothing that sends is in its dependency closure, and
//! `scripts/checks/check-crate-boundaries.py` holds that line (ADR 0009).
