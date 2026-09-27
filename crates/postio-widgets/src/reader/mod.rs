//! The message view both desktop apps draw (ADR 0043): the reading pane,
//! drawn by the reading renderer (spec 006).
//!
//! * [`view`] -- [`Reader`], the one public entry point: the body view on
//!   the renderer, and the header, notices, banners and chips around it.
//! * [`message_header`] -- the sender, recipients, subject and date strip
//!   above the body (#319).
//! * [`banner`] -- the remote-image, decode and unsubscribe notices, and the
//!   [`RemoteImageAllowList`] they consult.
//! * [`actions`] -- which verbs the reading pane's bars carry; a surface with
//!   a toolbar of its own passes none ([`Verbs`]).
//! * [`chips`] -- the attachments under a message.
//!
//! What only the classic app draws stays in postio-gtk: the conversation
//! rail, the parts panel, and the reading pane's place in the shell.

pub mod actions;
pub mod banner;
pub mod chips;
pub mod message_header;
mod notices;
pub mod view;

pub use actions::Verbs;
pub use message_header::MessageHeader;
pub use postio_body::{RemoteImages, quote, sanitize};
pub use postio_ui::allowlist;
pub use postio_ui::allowlist::RemoteImageAllowList;
pub use postio_ui::reader::parts::BlobSource;
pub use view::{Absent, HeldBack, Reader};
