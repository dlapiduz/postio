//! The reading pane: a hardened `WebView` for message bodies.
//!
//! Three issues, one module, because they build on each other in a straight
//! line:
//!
//! * [`postio-lu6`](view) — the `WebView` itself: JavaScript and network
//!   access off, inline images through a local [`scheme::BlobSource`],
//!   markup through [`postio_body::sanitize_body`], a click routed to the
//!   system browser instead of ever navigating the pane.
//! * [`postio-1bz`](postio_body::quote) — quoted-text folding, on the
//!   sanitized output `lu6` produces.
//! * [`postio-xxz`](banner) — the remote-image banner and its
//!   [`allowlist::RemoteImageAllowList`], both consuming the
//!   [`postio_body::Sanitized::remote_blocked`] flag the sanitizer already
//!   computes.
//!
//! [`view::Reader`] is the module's one public entry point; everything else
//! is a piece it assembles.
//!
//! # Where the markup half went
//!
//! `sanitize.rs` and `quote.rs` used to live here. They are now
//! [`postio_body`], because a body is not a GTK concept: the outgoing half
//! needs the same allowlist, and an allowlist the composer cannot reach is
//! one that gets copied. See ADR 0004. What stays here is WebKit — the view,
//! the `postio-cid:` scheme handler and the banner — and WebKit is the
//! frontend's business.
//!
//! The names are re-exported below so this module still reads as one thing
//! from the outside.

pub mod allowlist;
pub mod banner;
pub mod scheme;
pub mod view;

pub use allowlist::RemoteImageAllowList;
pub use postio_body::{RemoteImages, quote, sanitize};
pub use scheme::BlobSource;
pub use view::{Absent, Reader};
