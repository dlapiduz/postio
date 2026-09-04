//! The reading pane: a hardened `WebView` for message bodies.
//!
//! Several issues, one module, because they build on each other in a
//! straight line:
//!
//! * [`postio-lu6`](view) — the `WebView` itself: JavaScript and network
//!   access off, inline images through a local [`scheme::BlobSource`],
//!   markup through [`postio_body::sanitize_body`], a click routed to the
//!   system browser instead of ever navigating the pane.
//! * [`postio-1bz`](postio_body::quote) — quoted-text folding, on the
//!   sanitized output `lu6` produces.
//! * [`postio-xxz`](banner) — the remote-image banner and its
//!   [`allowlist::RemoteImageAllowList`], both consuming the
//!   counts the sanitizer already computes ([`postio_body::Sanitized`],
//!   split into ordinary images and likely trackers by [`HeldBack`]).
//! * `#319` — [`message_header`], the sender/recipients/subject/date strip
//!   above the banner: the reading pane's answer to the three questions a
//!   reader asks before it ever reaches the body.
//! * `#498` — [`actions`], the Reply/Reply all/Forward/Archive bar under the
//!   body: the pointer's way to reach the same four verbs the keyboard
//!   already had, and the reading pane's only click target of any kind
//!   before this.
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

pub mod actions;
pub mod allowlist;
pub mod banner;
pub mod message_header;
pub mod scheme;
pub mod view;

// The reading pane's bar is a `crate::widgets::ActionBar` now (#1002);
// `actions` still owns which four verbs it carries.
pub use allowlist::RemoteImageAllowList;
pub use message_header::MessageHeader;
pub use postio_body::{RemoteImages, quote, sanitize};
pub use scheme::BlobSource;
pub use view::{Absent, HeldBack, Reader};
