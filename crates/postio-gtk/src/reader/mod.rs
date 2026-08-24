//! The reading pane: a hardened `WebView` for message bodies.
//!
//! Three issues, one module, because they build on each other in a straight
//! line:
//!
//! * [`postio-lu6`](view) — the `WebView` itself: JavaScript and network
//!   access off, inline images through a local [`scheme::BlobSource`],
//!   markup through [`sanitize::sanitize_body`], a click routed to the
//!   system browser instead of ever navigating the pane.
//! * [`postio-1bz`](quote) — quoted-text folding, on the sanitized output
//!   `lu6` produces.
//! * [`postio-xxz`](banner) — the remote-image banner and its
//!   [`allowlist::RemoteImageAllowList`], both consuming the
//!   [`sanitize::Sanitized::remote_blocked`] flag `lu6`'s sanitizer already
//!   computes.
//!
//! [`view::Reader`] is the module's one public entry point; everything else
//! is a piece it assembles.

pub mod allowlist;
pub mod banner;
pub mod quote;
pub mod sanitize;
pub mod scheme;
pub mod view;

pub use allowlist::RemoteImageAllowList;
pub use sanitize::RemoteImages;
pub use scheme::BlobSource;
pub use view::Reader;
