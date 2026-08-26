//! Message bodies, in both directions.
//!
//! A mail client reads bodies and writes them, and until now only the reading
//! half existed — inside `postio-gtk`, beside the `WebView` that consumes it.
//! That put the one allowlist Postio has in the frontend, where a second
//! frontend could not reach it and where the *outgoing* half would have had
//! to grow a second copy. Two allowlists that must agree forever, with no
//! compiler to make them, is the failure [issue #30] was filed to prevent.
//!
//! So this crate owns the whole of what a body is:
//!
//! * [`sanitize`] — incoming markup, hardened before it reaches any view.
//! * [`quote`] — quoted-text folding, on that sanitized output.
//!
//! # Why its own crate and not `postio-model`
//!
//! [ADR 0004]. `ammonia` pulls `html5ever`, `markup5ever` and a generated tag
//! table, and `postio-model` is the crate every other crate in the workspace
//! waits on — four dependencies today, and nine crates behind it with no
//! opinion about HTML whatsoever. Putting an HTML parser there puts it in
//! front of every `cargo test -p <crate>` in the repository.
//!
//! This is a domain-rank leaf beside `postio-search`, and the same argument:
//! a pure library with no SQL and no toolkit, shared by whoever needs it,
//! rather than a capability buried in the frontend.
//!
//! # What is deliberately *not* here
//!
//! The `WebView`, the `postio-cid:` scheme handler and the remote-image
//! banner stay in `postio-gtk`. They are WebKit, and WebKit is the frontend's
//! business. What crosses the boundary is the string this crate produces.
//!
//! [issue #30]: https://github.com/dlapiduz/postio/issues/30
//! [ADR 0004]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0004-composer-document-model.md

pub mod document;
pub mod edit;
pub mod outgoing;
pub mod parse;
pub mod quote;
pub mod replying;
pub mod sanitize;

pub use document::{Block, ContentId, Document, HeadingLevel, Href, Inline, editor_image_src};
pub use edit::{EditHistory, EditStep};
pub use outgoing::{harden, render};
pub use parse::parse;
pub use quote::{fold_html_quotes, text_to_html};
pub use replying::{Placement, apply_signature, forwarded, quoted_reply};
pub use sanitize::{CID_SCHEME, RemoteImages, Sanitized, sanitize_body};
