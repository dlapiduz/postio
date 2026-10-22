//! The message view Postio draws (ADR 0043): the reading pane, drawn by the
//! reading renderer (spec 006).
//!
//! * [`view`] -- [`Reader`], the one public entry point: the body view on
//!   the renderer, and the header, notices, banners and chips around it.
//! * [`message_header`] -- the sender, recipients, subject and date strip
//!   above the body (#319).
//! * [`banner`] -- the remote-image, decode and unsubscribe notices, and the
//!   [`RemoteImageAllowList`] they consult.
//! * [`chips`] -- the attachments under a message.
//! * [`render_mode`] -- the line naming the treatment an HTML body is drawn
//!   in, for a reader that draws treatments ([`Reader::use_treatments`]).
//!
//! The verbs on a message are its owner's: Focus's open message draws them
//! in its own toolbar.

pub mod banner;
pub mod chips;
pub mod message_header;
mod notices;
pub mod render_mode;
pub mod view;

pub use message_header::MessageHeader;
pub use postio_body::{RemoteImages, quote, sanitize};
pub use postio_ui::allowlist;
pub use postio_ui::allowlist::RemoteImageAllowList;
pub use postio_ui::reader::parts::BlobSource;
pub use postio_ui::sender_treatments::SenderTreatments;
pub use view::{Absent, HeldBack, Reader};

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

thread_local! {
    /// Where the app's readers look up, and keep, a sender's treatment: set
    /// once by an app whose treatments live in the store ([`use_store_treatments`]).
    static TREATMENTS: RefCell<Option<Rc<dyn SenderTreatments>>> = RefCell::default();

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

/// Make `treatments` what every reader of this app looks a sender's
/// treatment up in and keeps it in (ADR 0048): the person's choice, read
/// from the store, rather than a section of the remote-image file. An app
/// that never calls this keeps them in that file, as its tests do; `None`
/// goes back to that.
pub fn use_store_treatments(treatments: Option<Rc<dyn SenderTreatments>>) {
    TREATMENTS.with(|slot| *slot.borrow_mut() = treatments);
}

/// What a reader built on `allowlist` (persisting to `path`) looks
/// treatments up in: the app's ([`use_store_treatments`]), else the list.
pub(crate) fn treatments_for(
    allowlist: &Rc<RefCell<RemoteImageAllowList>>,
    path: &Path,
) -> Rc<dyn SenderTreatments> {
    TREATMENTS
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(|| {
            Rc::new(FileTreatments {
                allowlist: Rc::clone(allowlist),
                path: path.to_owned(),
            })
        })
}

/// A sender's treatment kept in the remote-image file beside the grants: how
/// readers did it before treatments belonged to people, and what a reader
/// does when its app gave it nothing better.
struct FileTreatments {
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    path: PathBuf,
}

impl SenderTreatments for FileTreatments {
    fn treatment_for(&self, sender: &str) -> Option<postio_body::treatment::Treatment> {
        self.allowlist.borrow().treatment_for(sender)
    }

    fn set_treatment(&self, sender: &str, treatment: Option<postio_body::treatment::Treatment>) {
        let mut list = self.allowlist.borrow_mut();
        list.set_treatment(sender, treatment);
        if let Err(error) = list.save_to(&self.path) {
            tracing::warn!(%error, "could not save a sender's treatment");
        }
    }
}
