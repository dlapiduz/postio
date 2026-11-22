//! Message bodies in both directions: `sanitize` (incoming markup), `quote` (quoted-text folding)
//! and `own_text` (the newest message's own words).
//! A leaf crate apart from `postio-model`, so the HTML parser stays out of every crate's test build
//! (ADR 0004).
//! The `WebView`, the `postio-cid:` handler and the remote-image banner stay in the frontend.

pub mod document;
pub mod edit;
pub mod flowed;
pub mod forward;
mod hints;
pub mod links;
pub mod markdown;
pub mod narrow;
pub mod outgoing;
pub mod own_text;
pub mod parse;
pub mod quote;
pub mod reader_view;
pub mod replying;
pub mod sanitize;
pub mod styles;
pub mod treatment;

pub use document::{Block, ContentId, Document, HeadingLevel, Href, Inline, editor_image_src};
pub use edit::{EditHistory, EditStep};
pub use narrow::{Lost, Narrowed, narrow};
pub use outgoing::{harden, render};
pub use own_text::{own_line, own_text};
pub use parse::parse;
pub use quote::{fold_html_quotes, text_to_html};
pub use replying::{
    Placement, Presentation, Quoted, apply_signature, forwarded, quote_of, quoted_reply, requote,
};
pub use sanitize::{CID_SCHEME, Cap, RemoteImages, Sanitized, sanitize_body};
