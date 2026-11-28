//! Message bodies in both directions: `sanitize` (incoming markup), `quote` (quoted-text folding)
//! and `own_text` (the newest message's own words).
//! A leaf crate apart from `postio-model`, so the HTML parser stays out of every crate's test build
//! (ADR 0004).
//! The `WebView`, the `postio-cid:` handler and the remote-image banner stay in the frontend.

pub mod autolink;
pub mod document;
pub mod edit;
mod edit_ops;
pub mod flowed;
pub mod forward;
#[cfg(feature = "test-golden")]
pub mod golden;
mod hints;
pub mod links;
pub mod markdown;
pub mod narrow;
pub mod outgoing;
pub mod own_text;
pub mod parse;
pub mod position;
pub mod projection;
pub mod quote;
pub mod reader_view;
pub mod replying;
pub mod sanitize;
pub mod styles;
pub mod treatment;

pub use document::{
    Alignment, Block, Cell, ContentId, Document, HeadingLevel, Href, Inline, Language, ListItem,
    ListKind, Table, Task, WebHref, editor_image_src,
};
pub use edit::{EditHistory, EditStep, Undone};
pub use edit_ops::{
    Applied, BlockKind, BlockRange, Conversion, ConversionKind, EditIntent, Edited, Join, Mark,
    MarkSet, Unit, apply,
};
pub use narrow::{Lost, Narrowed, narrow};
pub use outgoing::{harden, message_body, render};
pub use own_text::{own_line, own_text};
pub use parse::parse;
pub use position::{DocPosition, Selection};
pub use projection::{Line, LineKind, LineStyle, Preedit, Projection, Run, project};
pub use quote::{fold_html_quotes, text_to_html};
pub use replying::{
    Placement, Presentation, Quoted, apply_signature, forwarded, quote_of, quoted_reply, requote,
};
pub use sanitize::{CID_SCHEME, Cap, RemoteImages, Sanitized, sanitize_body};
