//! The message view Postio draws (ADR 0043): [`Reader`] in [`view`] is the one public entry point;
//! the notices, banners and
//! chips around the body live in the sibling modules.

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
    static TREATMENTS: RefCell<Option<Rc<dyn SenderTreatments>>> = RefCell::default();

    /// Readers live on the GTK main thread, so this does too.
    static SHARED: RefCell<HashMap<PathBuf, Rc<RefCell<RemoteImageAllowList>>>> =
        RefCell::default();
}

/// Loaded the first time anything asks, then held and keyed by the file: an "Always allow" or
/// revoke reaches every reader,
/// open ones included.
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

/// An app that never calls this keeps treatments in the remote-image file, as its tests do; `None`
/// goes back to that (ADR 0048).
pub fn use_store_treatments(treatments: Option<Rc<dyn SenderTreatments>>) {
    TREATMENTS.with(|slot| *slot.borrow_mut() = treatments);
}

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
