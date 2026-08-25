//! The reading pane: a message the user picked, rendered.
//!
//! `postio-gtk` may not read the store, so a body cannot get into the reader
//! without this crate. The window mounts a `Reader` and knows how to show a
//! `MessageBody`; what it cannot do is get one, because that means SQLite
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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_core::bridge::EventSink;
use postio_gtk::feed::Feeds;
use postio_gtk::reader::{Absent, BlobSource};
use postio_gtk::sidebar::SyncStatus;
use postio_gtk::window::Window;
use postio_model::ids::{AttachmentId, BlobId};
use postio_model::{Attachment, MessageId};
use postio_runtime::Engine;
use postio_storage::Database;
use postio_storage::blob::BlobStore;
use postio_storage::repository::MessageRepository;

use crate::Wiring;

/// Fill the reading pane when a message is opened.
///
/// Hooked to the same activation the body backfill listens for, so opening a
/// message asks for its bytes and paints whatever is already local in the
/// same gesture.
///
/// `feeds` is where `ConnectionState` reaches this crate from: the sidebar
/// already renders it, so this reuses that seam (`Folders::status`,
/// `Folders::connect_status`) rather than opening a second one onto the
/// engine.
pub fn install(window: &Window, wiring: &Wiring, feeds: &Feeds) {
    // What the pane is showing, or is waiting to show. Set the instant a row
    // is activated rather than when the body lands, so a reply that arrives
    // late can tell it is late.
    let showing: Rc<Cell<Option<MessageId>>> = Rc::new(Cell::new(None));
    // What that message is made of, kept so a chip can open the tree without
    // going back to the store. Metadata only — see `Opened`.
    let opened: Rc<RefCell<Option<Opened>>> = Rc::new(RefCell::new(None));

    // A chip does not act, it asks: the panel is where the verbs live. Wired
    // once, and reads whichever message the pane is showing at the time.
    window.reader().connect_attachment(glib::clone!(
        #[weak]
        window,
        #[strong]
        opened,
        move |_node| {
            if let Some(opened) = opened.borrow().as_ref() {
                window.open_parts(&opened.root, &opened.parts);
            }
        }
    ));

    window.set_blob_source(cid_source(
        {
            let showing = showing.clone();
            move || showing.get()
        },
        wiring.database.clone(),
        wiring.blobs.clone(),
    ));

    // `s` in the parts panel. The panel has already run the portal dialog and
    // chosen the file; everything left is bytes, which is this crate's half.
    window.parts().connect_save(glib::clone!(
        #[weak]
        window,
        #[strong]
        showing,
        #[strong(rename_to = database)]
        wiring.database,
        #[strong(rename_to = blobs)]
        wiring.blobs,
        #[strong(rename_to = events)]
        wiring.events,
        #[strong(rename_to = engine)]
        wiring.engine,
        #[strong(rename_to = runtime)]
        wiring.runtime,
        move |node, file| {
            let (Some(attachment), Some(message)) = (node.attachment, showing.get()) else {
                return;
            };
            let (database, blobs, file) = (database.clone(), blobs.clone(), file.clone());
            let (events, engine) = (events.clone(), engine.get().cloned());
            let runtime = runtime.clone();
            let _ = &window;
            glib::spawn_future_local(async move {
                // `part_bytes` is runtime work, not main-context work: it may
                // ask the engine for a body that has not been downloaded and
                // then wait for it on a `tokio::time::sleep`. Awaiting that
                // here panicked with "there is no reactor running" -- the same
                // fault as postio-66, on the path that saves an attachment
                // whose message body is not local yet. So it goes over to the
                // runtime and answers on a channel, like every other crossing.
                let (sender, receiver) = async_channel::bounded(1);
                runtime.spawn(async move {
                    let bytes = part_bytes(&database, &blobs, engine, message, attachment).await;
                    let _ = sender.send(bytes).await;
                });
                let outcome = match receiver.recv().await {
                    Ok(Ok(bytes)) => write_part(&file, &bytes),
                    Ok(Err(reason)) => Err(reason),
                    Err(_) => Err("Postio's runtime stopped before that part arrived.".to_owned()),
                };
                if let Err(message) = outcome {
                    // Loud rather than silent: the user chose a filename and
                    // is entitled to know nothing arrived at it.
                    events.emit(postio_core::Event::Error { message });
                }
            });
        }
    ));

    let opener = Rc::new(PartOpener {
        database: wiring.database.clone(),
        blobs: wiring.blobs.clone(),
        events: wiring.events.clone(),
        runtime: wiring.runtime.clone(),
    });

    // `Ret` in the parts panel. `parts::previewable` says images and PDFs are
    // things the desktop already has a sensible viewer for, so those go
    // straight to it; everything else forces the "Open With" chooser, since
    // Postio has no better guess for a `.patch` than the user does.
    window.parts().connect_open(glib::clone!(
        #[weak]
        window,
        #[strong]
        showing,
        #[strong]
        opener,
        #[strong(rename_to = engine)]
        wiring.engine,
        move |node| {
            let always_ask = !postio_gtk::parts::previewable(&node.mime);
            opener.open_externally(
                &window,
                showing.get(),
                engine.get().cloned(),
                node,
                always_ask,
            );
        }
    ));

    // `x` in the parts panel -- "Open with…". Always forces the chooser: the
    // button says what it does, and guessing an app for it would make the
    // button lie.
    window.parts().connect_external(glib::clone!(
        #[weak]
        window,
        #[strong]
        showing,
        #[strong]
        opener,
        #[strong(rename_to = engine)]
        wiring.engine,
        move |node| {
            opener.open_externally(&window, showing.get(), engine.get().cloned(), node, true);
        }
    ));

    // `S` in the parts panel: the panel has already run the portal dialog and
    // chosen the folder. Every leaf goes through `export_part` once, named by
    // `parts::save_name` exactly as a single `s` names its own file.
    window.parts().connect_save_all(glib::clone!(
        #[weak]
        window,
        #[strong]
        showing,
        #[strong(rename_to = database)]
        wiring.database,
        #[strong(rename_to = blobs)]
        wiring.blobs,
        #[strong(rename_to = events)]
        wiring.events,
        #[strong(rename_to = engine)]
        wiring.engine,
        #[strong(rename_to = runtime)]
        wiring.runtime,
        move |folder| {
            let (Some(message), Some(into)) = (showing.get(), folder.path()) else {
                return;
            };
            let leaves: Vec<postio_gtk::parts::Node> = window
                .parts()
                .nodes()
                .into_iter()
                .filter(postio_gtk::parts::Node::is_leaf)
                .collect();
            let leaves_len = leaves.len();
            let (database, blobs) = (database.clone(), blobs.clone());
            let (events, engine) = (events.clone(), engine.get().cloned());
            let runtime = runtime.clone();
            glib::spawn_future_local(async move {
                // `save_all_parts` is runtime work for the same reason
                // `part_bytes` is: a part not yet downloaded waits on
                // `tokio::time::sleep`, which panics off the runtime.
                let (sender, receiver) = async_channel::bounded(1);
                let task_runtime = runtime.clone();
                task_runtime.spawn(async move {
                    let failed =
                        save_all_parts(&database, &blobs, engine, &into, message, &leaves).await;
                    let _ = sender.send(failed).await;
                });
                // Every part failed is the safe fallback if the runtime
                // vanished mid-batch -- see `write_part`'s analogous case.
                let failed = receiver.recv().await.unwrap_or(leaves_len);
                if failed > 0 {
                    // One toast for the whole batch rather than one per part:
                    // `S` can easily name a dozen parts, and a save that is
                    // mostly working does not need a dozen interruptions.
                    events.emit(postio_core::Event::Error {
                        message: format!(
                            "{failed} part{} could not be saved",
                            if failed == 1 { "" } else { "s" }
                        ),
                    });
                }
            });
        }
    ));

    // Dragging a part out to the desktop. Wired here rather than in
    // `export::install` because the panel says *which* part and this scope is
    // the only one that knows which message it belongs to.
    window.parts().connect_export({
        let showing = showing.clone();
        let database = wiring.database.clone();
        let blobs = wiring.blobs.clone();
        let engine = wiring.engine.clone();
        let runtime = wiring.runtime.clone();
        std::rc::Rc::new(move |node: postio_gtk::parts::Node| {
            let (database, blobs) = (database.clone(), blobs.clone());
            let (engine, runtime) = (engine.get().cloned(), runtime.clone());
            let message = showing.get();
            Box::pin(async move {
                let message = message.ok_or("There is no message open to take a part from")?;
                let into = crate::paths::export_dir();
                // On the runtime: this reads SQLite, may wait on a fetch, and
                // writes a file. None of that belongs on the UI thread, and
                // the drop is already asynchronous to GTK.
                let (send, receive) = async_channel::bounded(1);
                runtime.spawn(async move {
                    let outcome = crate::export::export_part(
                        &database, &blobs, engine, &into, message, &node,
                    )
                    .await;
                    let _ = send.send(outcome).await;
                });
                let path = receive
                    .recv()
                    .await
                    .map_err(|_| "The export did not finish".to_string())??;
                Ok(vec![gio::File::for_path(path)])
            })
        })
    });

    let database = wiring.database.clone();
    let blobs = wiring.blobs.clone();
    let runtime = wiring.runtime.clone();
    // One filler, two ways in.
    //
    // The cursor is the one that matters: `j` and `k` are how a mailbox is
    // read, and feeding the pane only from `connect_activated` -- Enter or a
    // double click -- is what left the column blank until somebody guessed
    // that Return was required (#70, Cause B). The maintainer settled it:
    // the preview follows the cursor and nothing waits for Return.
    //
    // Activation stays wired anyway, because it is not redundant. The cursor
    // reports only once the *user* has moved it, so on a window nobody has
    // touched the pane is deliberately empty -- and Enter on that window
    // still has to open the message under the cursor. Showing the same
    // message twice is harmless, so the overlap costs a store read and
    // nothing else.
    let parts = Rc::new(Fill {
        database,
        blobs,
        runtime,
        showing,
        opened,
        offline: Cell::new(is_offline(&feeds.folders.status())),
    });
    window.list().connect_cursor_moved(glib::clone!(
        #[weak]
        window,
        #[strong]
        parts,
        move |row| parts.fill(&window, row)
    ));
    window.list().connect_activated(glib::clone!(
        #[weak]
        window,
        #[strong]
        parts,
        move |row| parts.fill(&window, row)
    ));

    // Reconnecting (or losing the connection) has to repaint a pane that is
    // already showing a wait, not leave stale words on screen until the
    // cursor happens to move next -- see issue #117.
    feeds.folders.connect_status(glib::clone!(
        #[weak]
        window,
        #[strong]
        parts,
        move |status| {
            parts.offline.set(is_offline(status));
            parts.repaint_if_waiting(&window);
        }
    ));
}

/// Whether `status` says the engine has no connection at all right now.
///
/// Only [`ConnectionState::Offline`] counts: `Connecting` and `Failing` are
/// still trying, so a body already queued for backfill has not been given up
/// on the way `Offline` has.
fn is_offline(status: &SyncStatus) -> bool {
    matches!(status.state, ConnectionState::Offline)
}

/// Everything filling the reading pane needs, so the cursor and activation
/// can share one implementation rather than two that drift.
struct Fill {
    database: Database,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
    /// What the pane is showing, or is waiting to show.
    showing: Rc<Cell<Option<MessageId>>>,
    opened: Rc<RefCell<Option<Opened>>>,
    /// Whether the engine has no connection at all right now. Read by
    /// [`Fill::fill`] to pick `Absent::Offline` over `Absent::Partial`, and
    /// kept current by the `connect_status` handler `install` wires.
    offline: Cell<bool>,
}

impl Fill {
    /// Put `row`'s message in the pane, or say why it cannot be.
    fn fill(&self, window: &Window, row: postio_gtk::list::Row) {
        let message = row.id;
        let sender = row.from.as_ref().map(|from| from.address.clone());
        self.showing.set(Some(message));
        let offline = self.offline.get();

        let answer = crate::search::ask(&self.database, &self.runtime, {
            let blobs = self.blobs.clone();
            move |connection| {
                // One crossing for both. The parts are metadata the sync
                // already stored -- `BODYSTRUCTURE`, not bytes -- so asking
                // for them costs a row read and never a fetch.
                let body =
                    crate::compose::load_body_or_reason(connection, &blobs, message, offline);
                let (content_type, parts) = MessageRepository::new(connection)
                    .get(message)
                    .ok()
                    .flatten()
                    .map(|message| (message.content_type, message.attachments))
                    .unwrap_or_default();
                Some((body, content_type, parts))
            }
        });
        glib::spawn_future_local({
            let showing = self.showing.clone();
            let opened = self.opened.clone();
            let window = window.clone();
            async move {
                let Ok(Some((body, content_type, parts))) = answer.recv().await else {
                    return;
                };
                // Late. The cursor moved while the blob was read, and the
                // pane is showing something else now. This guard carries far
                // more weight than it used to: it used to filter double
                // clicks and now it filters a held-down `j`.
                if showing.get() != Some(message) {
                    return;
                }
                match body {
                    crate::compose::Body::Ready(body) => {
                        let root = root_type(content_type.as_deref(), &body, &parts);
                        window.reader().set_attachments(&root, &parts);
                        *opened.borrow_mut() = Some(Opened {
                            root,
                            parts,
                            absent: None,
                        });
                        window.show_message(&body, sender.as_deref());
                    }
                    crate::compose::Body::Absent(reason) => {
                        // The chips still go on. They are drawn from
                        // `BODYSTRUCTURE` metadata the sync already stored,
                        // so a message nothing has been fetched for can
                        // still say what came with it -- which is worth more
                        // than a blank pane, and is the one part of this
                        // state that is not a wait.
                        let root = root_type(
                            content_type.as_deref(),
                            &postio_model::MessageBody::default(),
                            &parts,
                        );
                        window.reader().set_attachments(&root, &parts);
                        *opened.borrow_mut() = Some(Opened {
                            root,
                            parts,
                            absent: Some(reason),
                        });
                        window.show_absent(reason);
                    }
                }
            }
        });
    }

    /// Repaint the pane in place if it is currently showing a wait whose
    /// wording depends on connectivity, now that connectivity changed.
    ///
    /// No store read: `opened` already holds the root type and parts from
    /// the last fill, and only the words explaining the wait change --
    /// `Missing` and `Empty` are not waits and are left alone.
    fn repaint_if_waiting(&self, window: &Window) {
        let mut opened = self.opened.borrow_mut();
        let Some(current) = opened.as_ref().and_then(|opened| opened.absent) else {
            return;
        };
        if !matches!(current, Absent::Partial | Absent::Offline) {
            return;
        }
        let reason = if self.offline.get() {
            Absent::Offline
        } else {
            Absent::Partial
        };
        if reason == current {
            return;
        }
        if let Some(opened) = opened.as_mut() {
            opened.absent = Some(reason);
        }
        drop(opened);
        window.show_absent(reason);
    }
}

/// What the message on screen is made of.
///
/// Held so activating a chip can open the tree without a second read, and so
/// the panel and the chip row cannot disagree about what they are describing.
struct Opened {
    /// The message's own content type, which is the tree's root row.
    root: String,
    /// Its parts, as `BODYSTRUCTURE` described them. Bytes not included.
    parts: Vec<Attachment>,
    /// `Some` when the pane is showing a wait rather than a real body, and
    /// which one -- so [`Fill::repaint_if_waiting`] can tell a connectivity
    /// change worth repainting from one that is not.
    absent: Option<Absent>,
}

/// The message's own content type — the row the parts tree hangs off.
///
/// # Read when it is there, derived otherwise
///
/// `BODYSTRUCTURE` says what it is and `postio-imap` records it in
/// [`Message::content_type`] at fetch time (`postio-roj4`), so `stored` is
/// the honest answer whenever a sync has actually filled it in. `stored` is
/// `None` for a row synced before that column existed and never refetched
/// since — the composer's own in-progress drafts too — and for those this
/// falls back to reconstructing a plausible shape from what *is* recorded: a
/// message with parts is `multipart/mixed`, one with two bodies is
/// `multipart/alternative`, and one with neither is whichever body it has.
///
/// The fallback can be wrong in exactly the case the real value fixes: a
/// `multipart/related` with inline images has parts, so it reads as
/// `multipart/mixed` here. That is a label on one row rather than a wrong
/// tree, which is why it was P3 rather than a bug.
///
/// [`Message::content_type`]: postio_model::Message::content_type
fn root_type(
    stored: Option<&str>,
    body: &postio_model::MessageBody,
    parts: &[Attachment],
) -> String {
    if let Some(content_type) = stored {
        return content_type.to_owned();
    }
    match (parts.is_empty(), body.text.is_some(), body.html.is_some()) {
        (false, _, _) => "multipart/mixed".to_owned(),
        (true, true, true) => "multipart/alternative".to_owned(),
        (true, false, true) => "text/html".to_owned(),
        _ => "text/plain".to_owned(),
    }
}

/// How long a save waits for a body it had to ask for.
///
/// Long enough for a slow server on a bad link, short enough that a save that
/// is never going to work says so while the user is still looking at it.
const BODY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// One part's bytes, fetched first if they are not on this machine yet.
///
/// # Why this is the seam rather than the save handler
///
/// `PartsPanel::save_part` runs the portal dialog itself and hands back the
/// file the user chose, so the only part worth testing is what happens next —
/// and that half is nothing to do with GTK. Keeping it here, taking store
/// handles and returning bytes, makes "saves a part that was never
/// downloaded, fetching it first" an ordinary async test over a mock server
/// instead of something that needs a display and a file chooser.
///
/// # Where a received part's bytes actually are
///
/// Not in `Attachment::blob_id`. That field is only ever filled on the way
/// *out* — `compose` puts a file the user attached into the blob store and
/// records its key — and nothing in the receive path writes it. What the
/// backfill stores is the whole raw message under `Message::raw_blob_id`, so
/// a received part is extracted from that rather than looked up.
///
/// So the fetch to wait for is the *message's*, not the part's: one round
/// trip brings every part with it, and asking again per attachment would be
/// the same bytes downloaded once per chip.
///
/// Returns `Err` rather than an empty file when the bytes cannot be had. A
/// zero-byte attachment on disk looks like a saved file and is not one.
pub(crate) async fn part_bytes(
    database: &Database,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    attachment: AttachmentId,
) -> Result<Vec<u8>, String> {
    // Resolved once, before anything is fetched, and deliberately.
    //
    // A fetch REPLACES the message's attachment rows -- the parser re-reads
    // the structure and `MessageRepository::update` writes the new set -- so
    // the `AttachmentId` the panel is holding does not survive it. The MIME
    // path does: `2` is `2` in every parse of the same bytes. So the id is
    // turned into a path here, while it still means something, and the path
    // is what is used on the far side.
    let (raw, part_id) = raw_and_part(database, message, attachment)?;
    let part_id = part_id.ok_or("That part has no place in the message to read it from")?;

    let raw = match raw {
        Some(raw) => raw,
        // Never downloaded. This is the one place in the reading pane allowed
        // to reach the network, and only because the user asked for these
        // bytes by name.
        None => {
            let engine =
                engine.ok_or("This account is not syncing, so that part cannot be fetched")?;
            // `request_body` puts the message at the front of the backfill and
            // returns as soon as it is queued -- `true` means "there was
            // something to fetch", not "here it is". The bytes land when the
            // engine's own loop claims the job, so the wait is ours.
            if !engine
                .request_body(message)
                .await
                .map_err(|error| error.message().to_string())?
            {
                return Err("There is nothing to fetch for that message".into());
            }
            wait_for_body(database, message).await?
        }
    };

    let bytes = blobs.get(&raw).map_err(|error| error.to_string())?;
    postio_model::mime::parse(&bytes)
        .parts
        .into_iter()
        .find(|part| part.attachment.part_id.as_deref() == Some(part_id.as_str()))
        .map(|part| part.content)
        .ok_or_else(|| "That part is not in the message the server sent".into())
}

/// Put one part's bytes where the user asked for them.
///
/// Replaces rather than appends: the dialog already asked about overwriting,
/// and a save that appended to an existing file would corrupt it silently.
fn write_part(file: &gio::File, bytes: &[u8]) -> Result<(), String> {
    file.replace_contents(
        bytes,
        None,
        false,
        gio::FileCreateFlags::REPLACE_DESTINATION,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
    .map_err(|error| format!("Could not save that part: {error}"))
}

/// What opening or "Open with…"-ing a part needs, bundled so the seam that
/// actually varies between the two -- `always_ask` -- does not have to travel
/// beside four things that never change per call.
struct PartOpener {
    database: Database,
    blobs: BlobStore,
    events: EventSink,
    runtime: tokio::runtime::Handle,
}

impl PartOpener {
    /// Fetch `node`'s bytes if it takes that, materialise them under
    /// [`crate::paths::export_dir`], and hand the result to the desktop's own
    /// launcher.
    ///
    /// Shared between `connect_open` and `connect_external`: both need the
    /// same bytes, fetched the same way
    /// [`export_part`](crate::export::export_part) already fetches them for a
    /// drag, and only disagree about whether the chooser is forced.
    ///
    /// A copy in the cache directory rather than a pipe or a temp file GTK
    /// reads once: the launched application owns the file from here, and some
    /// viewers (an image viewer's "next", a PDF reader's outline) hold it
    /// open well after launch returns.
    fn open_externally(
        &self,
        window: &Window,
        message: Option<MessageId>,
        engine: Option<Engine>,
        node: &postio_gtk::parts::Node,
        always_ask: bool,
    ) {
        let Some(message) = message else { return };
        let into = crate::paths::export_dir();
        let (window, node) = (window.clone(), node.clone());
        let (database, blobs, events, runtime) = (
            self.database.clone(),
            self.blobs.clone(),
            self.events.clone(),
            self.runtime.clone(),
        );
        glib::spawn_future_local(async move {
            let (sender, receiver) = async_channel::bounded(1);
            runtime.spawn(async move {
                let outcome =
                    crate::export::export_part(&database, &blobs, engine, &into, message, &node)
                        .await;
                let _ = sender.send(outcome).await;
            });
            let outcome = match receiver.recv().await {
                Ok(outcome) => outcome,
                Err(_) => Err("Postio's runtime stopped before that part arrived.".to_owned()),
            };
            match outcome {
                Ok(path) => launch(&window, &path, always_ask),
                Err(message) => {
                    events.emit(postio_core::Event::Error { message });
                }
            }
        });
    }
}

/// Save every part in `nodes` under `into`, fetching first when a part is
/// not local yet, and say how many could not be saved.
///
/// A count rather than which ones: `S` can easily name a dozen parts, and one
/// toast per failure would be worse than the save. Runtime work, not
/// main-context work, for the reason [`part_bytes`]'s own doc comment gives:
/// a part not yet downloaded waits on `tokio::time::sleep`, which panics off
/// the runtime.
pub(crate) async fn save_all_parts(
    database: &Database,
    blobs: &BlobStore,
    engine: Option<Engine>,
    into: &std::path::Path,
    message: MessageId,
    nodes: &[postio_gtk::parts::Node],
) -> usize {
    let mut failed = 0;
    for node in nodes {
        if crate::export::export_part(database, blobs, engine.clone(), into, message, node)
            .await
            .is_err()
        {
            failed += 1;
        }
    }
    failed
}

/// Hand `path` to the desktop's own opener.
///
/// `always_ask` forces the "Open With" chooser instead of the platform's own
/// default handler for the file's type -- what the panel's `x` promises by
/// calling itself "Open with…".
fn launch(window: &Window, path: &std::path::Path, always_ask: bool) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.set_always_ask(always_ask);
    launcher.launch(Some(window), gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            glib::g_warning!("postio", "could not open a part: {error}");
        }
    });
}

/// Wait for a queued body to land, or give up saying so.
///
/// Polling rather than listening: the engine announces arrivals on the event
/// stream, but that stream has exactly one reader — the window — and a second
/// consumer here would be a second place deciding what an event means. A save
/// the user is waiting on can afford to look.
///
/// The deadline is what turns a server that never answers into a sentence
/// rather than a spinner that never stops.
pub(crate) async fn wait_for_body(
    database: &Database,
    message: MessageId,
) -> Result<BlobId, String> {
    let deadline = std::time::Instant::now() + BODY_WAIT;
    loop {
        // A read that fails here is usually the writer we are waiting for
        // holding the table, so contention is a reason to look again rather
        // than to give up. Only the deadline ends this.
        match raw_blob(database, message) {
            Ok(Some(raw)) => return Ok(raw),
            Ok(None) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err("That part did not arrive in time — it is still \
                        downloading, so try again in a moment"
                .into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The raw-message blob key, and the MIME path of the wanted part.
///
/// Read together because both come off the same row, and the caller needs the
/// part's path whether or not the bytes have arrived yet.
fn raw_and_part(
    database: &Database,
    message: MessageId,
    attachment: AttachmentId,
) -> Result<(Option<BlobId>, Option<String>), String> {
    let row = read_message(database, message)?;
    let part_id = row
        .attachments
        .iter()
        .find(|part| part.id == attachment)
        .and_then(|part| part.part_id.clone());
    Ok((row.raw_blob_id, part_id))
}

/// Just the raw-message blob key. What the wait watches for.
pub(crate) fn raw_blob(database: &Database, message: MessageId) -> Result<Option<BlobId>, String> {
    Ok(read_message(database, message)?.raw_blob_id)
}

pub(crate) fn read_message(
    database: &Database,
    message: MessageId,
) -> Result<postio_model::Message, String> {
    let connection = database.connection().map_err(|error| error.to_string())?;
    MessageRepository::new(&connection)
        .get(message)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".into())
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

#[cfg(test)]
mod tests {
    //! The one thing about saving a part that is not GTK's problem: getting
    //! the bytes, including when they are not here yet.
    //!
    //! Nothing here needs a display, and nothing here touches the network —
    //! the engine is real and its backend is `MockBackend`, which is the seam
    //! `CLAUDE.md` names. `postio-6mza` proposed a trait in front of `Engine`
    //! for exactly this test and was closed once this turned out to work.

    use std::sync::Arc;

    use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
    use postio_model::MailboxRole;
    use postio_runtime::engine::{EngineParts, NetworkSource};
    use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
    use postio_storage::seed::seed_small;
    use postio_storage::test_support::TempDatabase;
    use postio_storage::{BlobStore, Database, test_support};

    use super::*;

    #[test]
    fn root_type_reads_the_stored_content_type_when_there_is_one() {
        // The case the derivation below gets wrong: a `multipart/related`
        // carrying inline images has parts, so the old heuristic always read
        // it as `multipart/mixed`. A stored value settles it outright.
        assert_eq!(
            root_type(
                Some("multipart/related"),
                &postio_model::MessageBody::default(),
                &[]
            ),
            "multipart/related"
        );
    }

    #[test]
    fn root_type_falls_back_to_derivation_when_nothing_is_stored() {
        // A row synced before `content_type` existed, or resynced and not
        // yet refetched -- the reconstruction `postio-roj4` describes.
        let with_html = postio_model::MessageBody {
            text: Some("plain".to_owned()),
            html: Some("<p>html</p>".to_owned()),
        };
        assert_eq!(
            root_type(None, &with_html, &[]),
            "multipart/alternative",
            "two bodies and no parts is the alternative case"
        );
        assert_eq!(
            root_type(None, &postio_model::MessageBody::default(), &[]),
            "text/plain",
            "neither body present falls back to plain"
        );
    }

    const BODY: &str = "the bytes that had to travel to get here";
    const ATTACHED: &str = "not a pdf";

    /// A store with mail in it, an engine over a mock server, and the id of a
    /// message whose parts are *not* downloaded.
    ///
    /// File-backed rather than [`test_support::memory`]: this world spawns a
    /// real [`Engine`] on a thread of its own, and an in-memory database has
    /// no WAL. Without it, the engine's writer and the test's own reads
    /// contend on the same shared-cache table lock, and `SQLITE_LOCKED` is
    /// not one `busy_timeout` retries away -- it is returned immediately,
    /// which is exactly the load-correlated panic #109 recorded from this
    /// test (`world` itself, and separately the read right after
    /// `part_bytes` returns). WAL is what production reads run under, so it
    /// is also the concurrency this test is supposed to be proving.
    fn world() -> (TempDatabase, BlobStore, Engine, MessageId) {
        let database = test_support::temp();
        let report = seed_small(&database, 11);
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.keep()).expect("a blob store");
        let (sink, _events) = postio_core::bridge::event_channel();

        let mut mailbox = MockMailbox::new(&inbox.path);
        for n in 1..=40 {
            // multipart/mixed, so part 2 is a real attachment the parser
            // will hand back with its own decoded bytes.
            mailbox = mailbox.message(MockMessage::new(
                format!(
                    "From: Ada Lovelace <ada@example.com>\r\n\
                     To: Postio <postio@example.net>\r\n\
                     Subject: part {n}\r\n\
                     Message-ID: <part-{n}@example.com>\r\n\
                     Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                     MIME-Version: 1.0\r\n\
                     Content-Type: multipart/mixed; boundary=\"edge\"\r\n\
                     \r\n\
                     --edge\r\n\
                     Content-Type: text/plain; charset=utf-8\r\n\
                     \r\n\
                     {BODY}\r\n\
                     --edge\r\n\
                     Content-Type: application/pdf\r\n\
                     Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
                     \r\n\
                     {ATTACHED}\r\n\
                     --edge--\r\n"
                )
                .into_bytes(),
            ));
        }

        // `seed` writes bodies as NotFetched and assigns no UID -- it exists
        // to fill a screenshot and knows nothing about any server. Give one
        // message the UID of a message the mock actually holds, so the engine
        // has something it can ask for; the rest are moved out of the way so
        // the unique index has no opinion.
        //
        // Done before `Engine::spawn` below, deliberately: the engine now
        // connects and reads these same tables the instant it starts (#109),
        // rather than five seconds later. Writing this fixture data
        // afterward would leave the engine's first discovery pass free to
        // run against whichever half of it had committed so far, which is
        // exactly the kind of thing a UID reassignment cannot survive being
        // wrong about.
        let connection = database.connection().expect("a connection");
        connection
            .execute(
                "UPDATE messages SET uid = id + 1000, uid_validity = 1 WHERE mailbox_id = ?1",
                [inbox.id.get()],
            )
            .expect("the fixture writes");
        let newest = MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: 1,
                after: None,
            })
            .expect("a page")
            .first()
            .expect("the inbox has mail")
            .id;
        connection
            .execute(
                "UPDATE messages SET uid = 1, uid_validity = 1 WHERE id = ?1",
                [newest.get()],
            )
            .expect("the fixture writes");
        drop(connection);

        let engine = Engine::spawn(EngineParts {
            account: report.account.id,
            database: (*database).clone(),
            blobs: blobs.clone(),
            backend: Arc::new(MockBackend::builder().mailbox(mailbox).build()),
            // Never dialled: nothing here queues a send.
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
            events: sink,
            retry: Default::default(),
            backfill: Default::default(),
            reconnect: Default::default(),
            watch: Default::default(),
            network: NetworkSource::default(),
        })
        .expect("an engine");

        (database, blobs, engine, newest)
    }

    /// The attachment row for the message the mock will serve, and its id.
    ///
    /// The store's row and the server's message have to describe the same
    /// part, which the seed cannot arrange on its own: it fills a screenshot
    /// from the corpus and knows nothing about any server.
    fn a_part_not_here(database: &Database, message: MessageId) -> AttachmentId {
        let connection = database.connection().expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut row = messages.get(message).expect("a read").expect("the message");
        assert!(
            row.raw_blob_id.is_none(),
            "the fixture already has this message's bytes, so this proves nothing"
        );

        let mut part = postio_model::Attachment::new(message, "application/pdf", 9);
        part.filename = Some("report.pdf".to_owned());
        // The MIME path the mock's message puts the attachment at.
        part.part_id = Some("2".to_owned());
        row.attachments = vec![part];
        update_with_retry(&messages, &mut row);

        messages
            .get(message)
            .expect("a read")
            .expect("the message")
            .attachments
            .first()
            .expect("the part was written")
            .id
    }

    /// As `MessageRepository::update`, but keeps trying for a moment.
    ///
    /// `world()`'s engine starts syncing the instant it is spawned, so this
    /// write can race a real background pass over the very same tables --
    /// `messages`, `attachments`. A `DatabaseLocked` there is the writer
    /// being raced holding the table, not a fault -- the same "look again"
    /// shape `wait_for_body` already uses for exactly this kind of
    /// contention. See #162.
    fn update_with_retry(messages: &MessageRepository, row: &mut postio_model::Message) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match messages.update(row) {
                Ok(()) => return,
                Err(error) if std::time::Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("the fixture writes: {error}"),
            }
        }
    }

    #[tokio::test]
    async fn a_part_nobody_has_is_fetched_before_it_is_saved() {
        // postio-v62's last criterion, arranged so it cannot pass without the
        // fetch: `part_bytes` is the only thing here that talks to the engine,
        // and the message has no raw blob until it does. A version of this
        // that called `request_body` itself first would prove only that bytes
        // already on disk can be read, which was never in doubt.
        let (database, blobs, engine, message) = world();
        let part = a_part_not_here(&database, message);

        let bytes = part_bytes(&database, &blobs, Some(engine), message, part)
            .await
            .expect("the part is fetched and handed back");

        assert_eq!(
            String::from_utf8_lossy(&bytes).trim(),
            ATTACHED,
            "the bytes handed back are the part's, not the message's"
        );
        // Not a bare `raw_blob(...).expect(...)`: the engine's backfill loop
        // is still writing other messages' rows on its own thread while this
        // reads. `wait_for_body` is what a caller waiting on exactly this
        // question already uses, the same helper `part_bytes` awaited above,
        // so re-checking through it costs nothing and asks nothing new of
        // the store.
        wait_for_body(&database, message).await.expect(
            "the fetch has to leave the message in the store, not only return \
             the part -- saving a second part must not go back to the server",
        );
    }

    #[tokio::test]
    async fn a_part_with_no_engine_says_so_rather_than_saving_nothing() {
        // The account is not syncing. Writing an empty file would look like a
        // successful save and would not be one.
        let (database, blobs, _engine, message) = world();

        let part = a_part_not_here(&database, message);

        let refused = part_bytes(&database, &blobs, None, message, part)
            .await
            .expect_err("there is no engine to fetch with");

        assert!(
            refused.contains("not syncing"),
            "the sentence has to say why: {refused}"
        );
    }

    #[tokio::test]
    async fn save_all_parts_fetches_what_it_needs_for_every_leaf() {
        // Same shape as `a_part_nobody_has_is_fetched_before_it_is_saved`, but
        // through the `S` path: nothing here is downloaded yet, so `S` must
        // fetch before it writes.
        let (database, blobs, engine, message) = world();
        let attachment = a_part_not_here(&database, message);
        let node = postio_gtk::parts::Node {
            part_id: "2".to_owned(),
            depth: 1,
            mime: "application/pdf".to_owned(),
            filename: Some("report.pdf".to_owned()),
            size: 9,
            downloaded: false,
            last: true,
            attachment: Some(attachment),
        };
        let into = tempfile::tempdir().expect("a save directory");

        let failed = save_all_parts(
            &database,
            &blobs,
            Some(engine),
            into.path(),
            message,
            &[node],
        )
        .await;

        assert_eq!(failed, 0, "the one leaf should have saved cleanly");
        assert_eq!(
            std::fs::read(into.path().join("report.pdf"))
                .expect("the file should exist")
                .trim_ascii(),
            ATTACHED.as_bytes(),
        );
    }

    #[tokio::test]
    async fn save_all_parts_counts_a_failure_without_abandoning_the_rest() {
        // A container has no bytes -- `export_part` refuses it -- but the
        // batch must still reach the leaf that comes after it, and the
        // caller has to be told one part did not make it.
        let (database, blobs, engine, message) = world();
        let attachment = a_part_not_here(&database, message);
        let container = postio_gtk::parts::Node {
            part_id: String::new(),
            depth: 0,
            mime: "multipart/mixed".to_owned(),
            filename: None,
            size: 0,
            downloaded: true,
            last: false,
            attachment: None,
        };
        let leaf = postio_gtk::parts::Node {
            part_id: "2".to_owned(),
            depth: 1,
            mime: "application/pdf".to_owned(),
            filename: Some("report.pdf".to_owned()),
            size: 9,
            downloaded: false,
            last: true,
            attachment: Some(attachment),
        };
        let into = tempfile::tempdir().expect("a save directory");

        let failed = save_all_parts(
            &database,
            &blobs,
            Some(engine),
            into.path(),
            message,
            &[container, leaf],
        )
        .await;

        assert_eq!(failed, 1, "the container is the only one that should fail");
        assert!(
            into.path().join("report.pdf").exists(),
            "the leaf after the failure must still be saved"
        );
    }
}
