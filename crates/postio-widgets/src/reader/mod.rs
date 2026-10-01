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
//! * [`render_mode`] -- the line naming the treatment an HTML body is drawn
//!   in, for a reader that draws treatments ([`Reader::use_treatments`]).
//!
//! What only the classic app draws stays in postio-gtk: the conversation
//! rail, the parts panel, and the reading pane's place in the shell.

pub mod actions;
pub mod banner;
pub mod chips;
pub mod message_header;
mod notices;
pub mod render_mode;
pub mod view;

pub use actions::Verbs;
pub use message_header::MessageHeader;
pub use postio_body::{RemoteImages, quote, sanitize};
pub use postio_ui::allowlist;
pub use postio_ui::allowlist::RemoteImageAllowList;
pub use postio_ui::reader::parts::BlobSource;
pub use view::{Absent, HeldBack, Reader};

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

thread_local! {
    /// The allow list each app's readers share, by the file it persists to.
    /// Readers live on the GTK main thread, so this does too.
    static SHARED: RefCell<HashMap<PathBuf, Rc<RefCell<RemoteImageAllowList>>>> =
        RefCell::default();
}

/// The remote-image allow list every reader of this app that persists to
/// `path` shares (specs/007-postio-focus T020).
///
/// Loaded from `path` the first time anything asks, then held: an "Always
/// allow" in one reader, or a revoke in the settings panel, is what every
/// other reader of the app reads next -- open ones included. Keyed by the
/// file, because an app's readers have that in common and an app writes one
/// file: that makes it one list per app, where each reader used to load a
/// copy of its own that went stale the moment another reader changed it.
pub fn shared_allowlist(path: &Path) -> Rc<RefCell<RemoteImageAllowList>> {
    SHARED.with(|shared| {
        Rc::clone(
            shared
                .borrow_mut()
                .entry(path.to_owned())
                .or_insert_with(|| Rc::new(RefCell::new(RemoteImageAllowList::load_from(path)))),
        )
    })
}
