//! Re-exports [`postio_core::blocking::now`] under this crate's own name.
//!
//! The implementation moved to `postio-core` so that `postio-widgets` could
//! reach it too (specs/007-postio-focus T022; ADR 0043 forbids
//! `postio-widgets` depending on this crate). Every existing caller in this
//! workspace still writes `postio_session::blocking::now`, and this keeps
//! that spelling true without a second copy of the runtime dance —
//! `postio_core::blocking`'s own doc comment has the reasoning and the
//! tests.

pub use postio_core::blocking::now;
