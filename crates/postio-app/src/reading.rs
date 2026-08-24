//! The reading pane: a message the user picked, rendered.
//!
//! `postio-gtk` may not read the store, so a body cannot get into the reader
//! without this crate. The window mounts a [`Reader`] and knows how to show a
//! [`MessageBody`]; what it cannot do is get one, because that means SQLite
//! and the blob store. This is the join.
//!
//! # The guard is the load-bearing part
//!
//! The blob read is asynchronous, and the cursor moves during it — holding `j`
//! is exactly that, many times a second. So a body can arrive after the user
//! has already moved on, and painting it would put one message's body under
//! another's header. Every render is therefore checked against what the pane
//! is *currently* showing, and a stale answer is dropped.
//!
//! This is why the pane does not flicker between messages when a key is held.
//! `postio-b5` found it in the search preview; it is the same hazard here and
//! the same answer.
//!
//! # Nothing here reaches the network
//!
//! A body that has not been fetched yet simply does not draw. The engine
//! backfills separately — `lib.rs::fetch_what_is_opened` moves the opened
//! message to the front of that queue — and the pane fills in when it lands.
//! Waiting on a socket to paint would put the UI on the network, which is the
//! one thing the whole local-first shape exists to prevent.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use postio_gtk::reader::BlobSource;
use postio_gtk::window::Window;
use postio_model::MessageId;
use postio_storage::Database;
use postio_storage::blob::BlobStore;
use postio_storage::repository::MessageRepository;

use crate::Wiring;

/// Fill the reading pane when a message is opened.
///
/// Hooked to the same activation the body backfill listens for, so opening a
/// message asks for its bytes and paints whatever is already local in the
/// same gesture.
pub fn install(window: &Window, wiring: &Wiring) {
    // What the pane is showing, or is waiting to show. Set the instant a row
    // is activated rather than when the body lands, so a reply that arrives
    // late can tell it is late.
    let showing: Rc<Cell<Option<MessageId>>> = Rc::new(Cell::new(None));

    window.set_blob_source(cid_source(
        {
            let showing = showing.clone();
            move || showing.get()
        },
        wiring.database.clone(),
        wiring.blobs.clone(),
    ));

    let database = wiring.database.clone();
    let blobs = wiring.blobs.clone();
    let runtime = wiring.runtime.clone();
    window.list().connect_activated(glib::clone!(
        #[weak]
        window,
        move |row| {
            let message = row.id;
            let sender = row.from.as_ref().map(|from| from.address.clone());
            showing.set(Some(message));

            let answer = crate::search::ask(&database, &runtime, {
                let blobs = blobs.clone();
                move |connection| Some(crate::compose::load_body(connection, &blobs, message))
            });
            glib::spawn_future_local({
                let showing = showing.clone();
                async move {
                    let Ok(Some(body)) = answer.recv().await else {
                        return;
                    };
                    // Late. The cursor moved while the blob was read, and the
                    // pane is showing something else now.
                    if showing.get() != Some(message) {
                        return;
                    }
                    window.show_message(&body, sender.as_deref());
                }
            });
        }
    ));
}

/// Where a rendered message resolves its `cid:` parts from.
///
/// # Scoped to one message on purpose
///
/// A `Content-ID` is only meaningful inside the message that declares it, so
/// resolving one globally would let a sender address another sender's parts.
/// [`BlobSource`] carries no message, so the caller supplies `showing` and
/// this asks it at the moment the scheme handler runs — which is also what
/// makes it correct while the pane is changing.
///
/// # A part that is not here does not draw
///
/// The hardened view has network access off, so a part whose bytes are not
/// already on this machine resolves to nothing. That is the privacy
/// commitment working rather than a failure to handle: a remote fetch here
/// would be the tracking pixel the reader spent so much effort blocking,
/// arriving through the back door.
///
/// Shared with the search preview, which has the same problem with a
/// different notion of "the message on screen" — hence the closure rather
/// than a widget.
pub(crate) fn cid_source(
    showing: impl Fn() -> Option<MessageId> + 'static,
    database: Database,
    blobs: BlobStore,
) -> Rc<dyn BlobSource> {
    Rc::new(move |content_id: &str| {
        let message = showing()?;
        let connection = database.connection().ok()?;
        let part = MessageRepository::new(&connection)
            .get(message)
            .ok()??
            .attachments
            .into_iter()
            .find(|part| part.content_id.as_deref() == Some(content_id))?;
        let bytes = blobs.get(&part.blob_id?).ok()?;
        Some((bytes, part.mime_type))
    })
}
