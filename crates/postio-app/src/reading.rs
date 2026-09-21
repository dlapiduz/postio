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
//!
//! "Fills in when it lands" is [`Fill::body_arrived`], and it was the missing
//! half of that sentence until #396: the engine announced every body it
//! committed and nothing in the workspace listened, so a pane left showing
//! "Downloading this message" stayed that way until an unrelated redraw
//! corrected it. The arrival is pushed, never polled — the same rule the rest
//! of this file follows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_core::bridge::EventSink;
use postio_gtk::feed::Feeds;
use postio_gtk::reader::Absent;
use postio_gtk::sidebar::SyncStatus;
use postio_gtk::window::Window;
use postio_model::address::EmailAddress;
use postio_model::ids::{AttachmentId, BlobId};
use postio_model::{Attachment, Message, MessageId};
use postio_runtime::Engine;
use postio_storage::Store;
use postio_storage::blob::BlobStore;
use postio_storage::repository::MessageRepository;

use crate::Wiring;

/// Which message the reading pane is showing, or is waiting to show.
///
/// Shared rather than private because it is the answer to two questions that
/// must never differ: what to paint, and what `e` replies to. `compose.rs`
/// kept a second copy fed by `List::connect_activated` alone, so a session
/// spent reading with `j` left it `None` and reply, reply-all and forward
/// were all inert (#325). One cell, created by the composition root and
/// handed to both, is what makes that class of drift unrepresentable.
pub type Showing = Rc<Cell<Option<MessageId>>>;

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
/// The accounts the reading pane may have to name, in the order
/// `AppState::accounts` uses, so an account's hue matches the sidebar's.
///
/// Read once at install rather than per message: it is a handful of rows that
/// change only when an account is added or removed, and the reading pane is
/// on the interaction budget.
///
/// Empty when the store holds one account or none — which is what makes the
/// account line invisible for everybody who has not configured a second one
/// (#185). Not "hidden by a flag": there is nothing to say.
async fn accounts_to_name(
    database: &postio_storage::Store,
) -> Vec<(postio_model::AccountId, String)> {
    let Ok(connection) = database.connect().await else {
        return Vec::new();
    };
    let accounts = postio_storage::repository::AccountRepository::new(&connection)
        .list()
        .await
        .unwrap_or_default();
    if accounts.len() < 2 {
        return Vec::new();
    }
    accounts
        .into_iter()
        .map(|account| (account.id, account.display_name))
        .collect()
}

pub async fn install(window: &Window, wiring: &Wiring, feeds: &Feeds, showing: Showing) {
    // See `accounts_to_name`: empty in the single-account case, which is the
    // common one, and then this costs a length check per message.
    let named_accounts: Rc<Vec<(postio_model::AccountId, String)>> =
        Rc::new(accounts_to_name(&wiring.database).await);
    // `showing` is what the pane is showing, or is waiting to show. Set the
    // instant the cursor reaches a row rather than when the body lands, so a
    // body that arrives late can tell it is late. `compose.rs` reads the
    // same cell -- see [`Showing`].
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

    // `p`: the same destination as clicking a chip, for a message the
    // keyboard is on with no chip to click at all.
    window.reader().connect_parts_requested(glib::clone!(
        #[weak]
        window,
        #[strong]
        opened,
        move || {
            if let Some(opened) = opened.borrow().as_ref() {
                window.open_parts(&opened.root, &opened.parts);
            }
        }
    ));

    // #971: the banner only asks — logging the activation means SQLite,
    // which `postio-gtk` may not touch. `showing` is read at click time
    // rather than captured with the message, because the two can only ever
    // agree: the banner names whichever message is currently on screen.
    window.reader().connect_unsubscribe_activated({
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        let showing = showing.clone();
        move |list_identifier| {
            postio_session::blocking::now(async {
                let Some(message) = showing.get() else {
                    return;
                };
                let list_identifier = list_identifier.to_owned();
                let _ = crate::search::ask(&database, &runtime, move |connection| async move {
                    let account_id = MessageRepository::new(&connection)
                        .get(message)
                        .await
                        .ok()
                        .flatten()?
                        .account_id;
                    let mut activation = postio_model::UnsubscribeActivation::new(
                        account_id,
                        list_identifier,
                        chrono::Utc::now(),
                    );
                    postio_storage::repository::UnsubscribeRepository::new(&connection)
                        .record(&mut activation)
                        .await
                        .ok()
                });
            })
        }
    });

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
                // One toast for the whole batch rather than one per part:
                // `S` can easily name a dozen parts, and a save that is
                // mostly working does not need a dozen interruptions.
                //
                // The sentence is `postio_ui::reader::parts::save_all_failure`'s
                // rather than this closure's, because the macOS boundary
                // reports the same partial save and two frontends phrasing it
                // separately is how they come to disagree about it. `None` is
                // what "nothing failed" looks like, so the test for it is the
                // same expression as the wording.
                if let Some(sentence) = postio_gtk::parts::save_all_failure(failed) {
                    events.emit(postio_core::Event::Error { message: sentence });
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
    let showing_for_conversation = showing.clone();
    let parts = Rc::new(Fill {
        database,
        runtime,
        showing,
        opened,
        named_accounts,
        offline: Rc::new(Cell::new(is_offline(&feeds.folders.status()))),
        queued: Cell::new(false),
        aimed: Cell::new(None),
        engine: wiring.engine.clone(),
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

    // The thread column's cursor is *not* wired to this filler.
    //
    // #436 wired it here, and was right to: the column drove the reading
    // pane, which showed one message. ADR 0015 Q4 changed what the column is
    // for. It is now an index into the conversation pane, so its cursor
    // scrolls that pane rather than replacing the reader's contents --
    // wired in `Window::conversation`, where both surfaces are in scope.
    //
    // Leaving the old wiring in place would not look broken, which is why it
    // is worth a paragraph: the single reader is hidden while a conversation
    // is open, so this would have gone on reading a body from the store on
    // every `j` and rendering it where nobody could see it.

    // The conversation pane (ADR 0015 Q4, #308).
    //
    // Its readers are built here rather than by the widget because only the
    // window has the blob source and the allow-list path, and only this
    // module knows how a body is loaded. The pane decides *how many* to ask
    // for; this decides what one contains.
    // ADR 0032, Accepted 2026-09-09 (#1316): a thread is one document in one
    // `WebView`, not a stack of readers.
    //
    // This was `if std::env::var_os("POSTIO_ONE_DOCUMENT").is_some()` while
    // the ADR was Proposed, and the variable's own comment said why: "an
    // experiment with a decision still to be made, and `config.toml` is where
    // settled choices live." The decision is made, so there is no variable and
    // no second shape to fall back to -- there are no deployed installs to
    // keep a fallback for, and a code path nothing exercises is a code path
    // that rots.
    //
    // The ADR was accepted **without** its own screen-reader gate being met;
    // that is recorded there and the pass is #1424. If Orca finds the HTML
    // worse than the widget tree it replaced, the answer is to fix the HTML,
    // not to reach for a stacked pane nobody has run in months.
    {
        window.conversation().set_one_document(true);
        window.conversation().connect_thread_opened({
            let fill = Rc::clone(&parts);
            let window = glib::object::ObjectExt::downgrade(window);
            move |rows| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                fill.fill_thread(&window.conversation(), rows);
            }
        });
    }

    window.conversation().set_reader_factory({
        // Weak, for the reason `install_run` states in `search.rs`: the
        // conversation pane is a child the window owns, and a strong clone
        // stored in its factory is a cycle that keeps the window alive for
        // the life of the process (#1072). A window that has gone has
        // nothing to build a reader with, which is what `None` says.
        let window = glib::object::ObjectExt::downgrade(window);
        let parts = Rc::clone(&parts);
        move |message| {
            let window = window.upgrade()?;
            let reader = window.new_reader();
            // The reader's own sender/subject/date stay hidden (#308): the
            // entry above it already carries that line. Recipients (#487)
            // are the one part of this header the entry does not draw, so
            // the header widget itself stays visible and only its identity
            // portion is hidden — `fill_reader` fills in To/Cc once the
            // envelope has loaded.
            reader.header().set_identity_visible(false);
            // Same reason (#822): the entry below the header already draws
            // its own Reply/Reply all/Forward row
            // (`conversation::ConversationView::build_entry`), deliberately
            // without Archive — every other verb belongs to the
            // conversation, not to one message in the stack. This reader's
            // own action bar would duplicate it, in a different style, with
            // a fourth button nothing here should offer per-message, and its
            // clicks reach nothing anyway: `new_reader` never wires
            // `connect_command` the way `Window::reader` does for the
            // standalone pane.
            reader.set_actions_visible(false);
            // Hidden until it has something to draw, so an expanded message
            // whose body is still being read is a header rather than a white
            // rectangle pretending to be a message.
            reader.widget().set_visible(false);
            parts.fill_reader(&reader, message);
            Some(reader)
        }
    });

    // `e`, `E` and `f` reply to whatever is current, and inside a
    // conversation that is the focused message. Writing it into the same
    // `showing` cell the single-message pane writes means the reply source
    // has one answer rather than two that can disagree -- which is what #325
    // was, and what `compose::install_reply_source` documents at length.
    window.conversation().connect_focus_changed({
        let showing = showing_for_conversation;
        move |message| showing.set(Some(message))
    });

    // The per-message verbs (ADR 0015 Q4). Reply, reply-all and forward are
    // the only ones drawn on a message; everything else in this pane acts on
    // the conversation.
    //
    // Focus first, then dispatch. The composer resolves what it is answering
    // through `connect_reply_source`, which reads `showing` -- so making the
    // clicked message current *is* how the reply gets aimed at it, and it
    // aims the keyboard's `e` at the same message in the same movement.
    // Naming the message in the command as well costs nothing and keeps the
    // invocation honest about what was asked for.
    window.conversation().connect_reply(glib::clone!(
        #[weak]
        window,
        move |message, all| {
            window.conversation().focus_message(message);
            window.act(if all {
                postio_core::Command::ReplyAll {
                    message: Some(message),
                }
            } else {
                postio_core::Command::Reply {
                    message: Some(message),
                }
            });
        }
    ));
    window.conversation().connect_forward(glib::clone!(
        #[weak]
        window,
        move |message| {
            window.conversation().focus_message(message);
            window.act(postio_core::Command::Forward {
                message: Some(message),
            });
        }
    ));

    // Resting on a message in the conversation reads it, on the same rule as
    // resting on a row in the list (#71). Opening a conversation does not
    // read it: the timer starts when focus lands and is cancelled when it
    // moves, so walking the index passes over messages without marking them.
    window.conversation().connect_dwelled(glib::clone!(
        #[weak]
        window,
        move |message| window.act(postio_core::Command::MarkReadOnDwell { message })
    ));

    // A body -- or an attachment's bytes -- arriving for the message on
    // screen (#396).
    //
    // The engine has emitted `BodyLoaded` since it was written, and until
    // this line nothing in the workspace acted on it: the pane a person was
    // watching went on showing "Downloading this message" after the bytes
    // were local, until some unrelated redraw happened to correct it. The
    // reading pane is fed from the same one call every other pane is
    // (`Feeds::apply`); what it could not be is fed from inside `postio-gtk`,
    // which may not read a body. See `Feeds::connect_event`.
    feeds.connect_event({
        let parts = Rc::clone(&parts);
        let window = window.downgrade();
        move |event| {
            let postio_core::Event::BodyLoaded { message, .. } = event else {
                return;
            };
            if let Some(window) = window.upgrade() {
                parts.body_arrived(&window, *message);
            }
        }
    });

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

/// What fetches a body that is not here yet, for a closure that has outlived
/// the [`Fill`] it came from — see [`Fill::fetcher`].
#[derive(Clone)]
struct BodyFetcher {
    engine: postio_session::refresh::EngineSlot,
    runtime: tokio::runtime::Handle,
}

impl BodyFetcher {
    /// Ask the engine for `message`'s body if `loaded` says it is not here.
    ///
    /// [`Absent::Partial`] is "headers synced, body not fetched"; the
    /// backfill reaches it eventually, and this is what makes opening it now
    /// jump the queue. Nothing to do without an engine — a window over a
    /// store nobody is syncing — or for any other absence, which no fetch
    /// would change.
    fn request_if_partial(&self, message: MessageId, loaded: &Loaded) {
        if let crate::compose::Body::Absent(Absent::Partial) = &loaded.body
            && let Some(engine) = self.engine.get().cloned()
        {
            self.runtime.spawn(async move {
                let _ = engine.request_body(message).await;
            });
        }
    }
}

/// What draws the single reading pane, for a closure that has outlived the
/// [`Fill`] it came from — see [`Fill::painter`]: the cells [`paint`] reads,
/// and the window it paints into.
struct Painter {
    window: Window,
    showing: Showing,
    opened: Rc<RefCell<Option<Opened>>>,
    named_accounts: Rc<Vec<(postio_model::AccountId, String)>>,
    offline: Rc<Cell<bool>>,
}

impl Painter {
    /// Whether the pane still wants `message` by the time its read answered.
    ///
    /// Late is the normal case, not the edge case: the cursor moved while
    /// the blob was read, and the pane is showing something else now. This
    /// guard carries far more weight than it used to — it used to filter
    /// double clicks and now it filters a held-down `j`.
    fn still_showing(&self, message: MessageId) -> bool {
        self.showing.get() == Some(message)
    }

    /// Draw `loaded` as `message` — see [`paint`].
    fn paint(&self, message: MessageId, loaded: Loaded) {
        paint(
            &self.window,
            &self.opened,
            &self.named_accounts,
            &self.offline,
            message,
            loaded,
        );
    }

    /// Redraw the parts panel from what the pane just opened, when it is up.
    ///
    /// Its chips are drawn from the same attachment rows the reader's are,
    /// and `Node::downloaded` genuinely changes at runtime (#377), so a chip
    /// that said "download" has to stop saying it. Whatever `opened` holds
    /// is the right tree: the panel owns the keyboard while it is up
    /// (`Context::Parts`), so the cursor cannot have moved to another
    /// message underneath it.
    fn refresh_parts(&self) {
        let panel = self.window.parts();
        if panel.is_visible()
            && let Some(opened) = self.opened.borrow().as_ref()
        {
            panel.update_parts(&opened.root, &opened.parts);
        }
    }
}

/// Everything filling the reading pane needs, so the cursor and activation
/// can share one implementation rather than two that drift.
struct Fill {
    database: Store,
    runtime: tokio::runtime::Handle,
    /// What the pane is showing, or is waiting to show.
    showing: Showing,
    opened: Rc<RefCell<Option<Opened>>>,
    /// The accounts the pane may have to name — see [`accounts_to_name`].
    /// Empty in the single-account case, which is what keeps the account
    /// line off screen for anybody who has not configured a second one.
    named_accounts: Rc<Vec<(postio_model::AccountId, String)>>,
    /// Whether the engine has no connection at all right now. Read by
    /// [`Fill::fill`] to pick `Absent::Offline` over `Absent::Partial`, and
    /// kept current by the `connect_status` handler `install` wires.
    /// Shared, because a fill reads it twice: once to ask the store, and
    /// again when the answer comes back. See [`Fill::waiting_reason`].
    offline: Rc<Cell<bool>>,
    /// Whether a repaint of the *single* pane is already queued for this
    /// turn of the main loop — see [`Fill::body_arrived`].
    queued: Cell<bool>,
    /// Which message the *single* reading pane was last aimed at, and so
    /// which one asking again would be asking for twice — see [`Fill::fill`].
    ///
    /// Deliberately not [`Fill::showing`], which answers a different
    /// question. `showing` is what the reply verbs aim at, and the
    /// conversation pane writes into it whenever focus moves between the
    /// messages of a thread; it is also never cleared, so after a folder
    /// change it still names a message this pane is no longer displaying.
    /// Either of those would make it the wrong thing to skip on.
    aimed: Cell<Option<MessageId>>,
    /// The engine, so showing a message whose body is not here yet is what
    /// fetches it.
    ///
    /// The backfill reaches every body eventually, but "eventually" is a
    /// queue tens of thousands of messages long on a first sync — so opening
    /// one has to jump it to the front rather than wait its turn.
    /// [`Absent::Partial`] is precisely "headers synced, body not fetched",
    /// and its own doc says a `request_body` is what leaves that state; this
    /// is the caller that keeps that promise. A slot rather than an `Engine`,
    /// because the pane is built before an account's engine is adopted
    /// (`adopt_engine`), and reads empty until it is.
    engine: postio_session::refresh::EngineSlot,
}

impl Fill {
    /// Render one message into a reader of its own, for the conversation
    /// pane (ADR 0015 Q4, #308).
    ///
    /// The same read as [`Fill::fill`] — the same body loader, the same
    /// `root_type`, so the two cannot drift on what a message is — rendered
    /// into a given [`Reader`](postio_gtk::reader::Reader) instead of into the
    /// window's one.
    ///
    /// # Why the late-arrival guard is different
    ///
    /// [`Fill::fill`] guards on `showing`, because the pane shows one message
    /// and a held-down `j` means the answer that comes back is often for a
    /// message nobody is looking at any more. A conversation entry shows one
    /// *fixed* message for as long as it exists, so there is nothing to race:
    /// the reader handed in either still exists, in which case the answer is
    /// still its answer, or it has been dropped and rendering into it is
    /// harmless. Reusing `showing` here would be worse than useless — it
    /// would discard every message in the stack except the focused one.
    /// The reason to show for a body that is not here, decided **now**.
    ///
    /// A fill captures the connection state, crosses to a worker to read the
    /// store, and comes back a moment later — and the connection can move in
    /// between. Applying the captured answer then undoes what
    /// [`repaint_if_waiting`](Self::repaint_if_waiting) just did: coming
    /// online flips the plate to `Partial`, an in-flight fill lands with the
    /// `Offline` it set out with, and the pane goes back to promising a
    /// backfill that is in fact already running.
    ///
    /// Only the two waiting states are re-derived. `Missing` and `Empty` are
    /// facts about the message rather than about the link, and a reconnection
    /// does not change them.
    ///
    /// [`repaint_if_waiting`]: Self::repaint_if_waiting
    fn waiting_reason(offline: &Cell<bool>, reason: Absent) -> Absent {
        match reason {
            Absent::Partial | Absent::Offline => {
                if offline.get() {
                    Absent::Offline
                } else {
                    Absent::Partial
                }
            }
            settled => settled,
        }
    }

    /// Everything the pane needs about one message, read in a single
    /// crossing.
    ///
    /// One read serves the three callers — the pane following the cursor, a
    /// conversation entry, and a repaint when a body lands — so none of them
    /// can drift on what a message is.
    fn read(&self, message: MessageId) -> async_channel::Receiver<Option<Loaded>> {
        let offline = self.offline.get();
        crate::search::ask(&self.database, &self.runtime, {
            move |connection| async move {
                // One crossing for all of it. The parts are metadata the sync
                // already stored -- `BODYSTRUCTURE`, not bytes -- so asking
                // for them costs a row read and never a fetch.
                let body = crate::compose::load_body_or_reason(&connection, message, offline).await;
                let fetched = MessageRepository::new(&connection)
                    .get(message)
                    .await
                    .ok()
                    .flatten();
                let (content_type, parts) = fetched
                    .as_ref()
                    .map(|message| (message.content_type.clone(), message.attachments.clone()))
                    .unwrap_or_default();
                let sender = fetched
                    .as_ref()
                    .and_then(|message| message.from.first().map(|from| from.address.clone()));
                let list_identifier = fetched.as_ref().and_then(list_identifier);
                let send_state = MessageRepository::new(&connection)
                    .send_state(message)
                    .await
                    .unwrap_or_default();
                let envelope = fetched.map(Envelope::from);
                Some(Loaded {
                    body,
                    content_type,
                    parts,
                    envelope,
                    sender,
                    send_state,
                    list_identifier,
                })
            }
        })
    }

    /// Read `message` and, when the answer lands back on the main loop, hand
    /// it to `then`.
    ///
    /// Every fill is this shape — one crossing through [`read`](Self::read),
    /// awaited where the widgets are — and each caller used to spell the
    /// crossing out itself. A read that comes back empty (the message went
    /// away underneath it) is simply not painted.
    fn read_then(&self, message: MessageId, then: impl FnOnce(Loaded) + 'static) {
        // Started before the spawn: `read` borrows `self`, and a `'static`
        // task cannot carry that borrow.
        let answer = self.read(message);
        glib::spawn_future_local(async move {
            let Ok(Some(loaded)) = answer.recv().await else {
                return;
            };
            then(loaded);
        });
    }

    /// See [`BodyFetcher`].
    fn fetcher(&self) -> BodyFetcher {
        BodyFetcher {
            engine: self.engine.clone(),
            runtime: self.runtime.clone(),
        }
    }

    /// See [`Painter`].
    fn painter(&self, window: &Window) -> Painter {
        Painter {
            window: window.clone(),
            showing: self.showing.clone(),
            opened: self.opened.clone(),
            named_accounts: Rc::clone(&self.named_accounts),
            offline: Rc::clone(&self.offline),
        }
    }

    /// Fetch every body in a thread, for one-document mode (ADR 0032, #1316).
    ///
    /// The stacked pane fetches a body when a message is expanded, and
    /// `fill_reader` is where that lands. One document has no expansions to
    /// hang it on: the whole thread is drawn at once, so the whole thread is
    /// asked for at once and each body is handed to the pane as it arrives.
    ///
    /// Still one crossing per message, and still through `read`, so a body
    /// that is not on this machine reports the same absence it would in the
    /// stack — a message waiting for its body draws collapsed with its
    /// preview rather than as an empty box.
    fn fill_thread(
        &self,
        pane: &postio_gtk::conversation::ConversationView,
        rows: Vec<postio_gtk::list::Row>,
    ) {
        for row in rows {
            let pane = pane.clone();
            let fetch = self.fetcher();
            self.read_then(row.id, move |loaded| {
                // Not here yet. Fetch it, so a conversation the backfill
                // has not reached fills in as it is opened rather than
                // staying a stack of empty headers -- `body_arrived`
                // draws it into this same pane when it lands.
                fetch.request_if_partial(row.id, &loaded);
                // The envelope first, and separately from the body: a
                // message can have one without the other, and who it went
                // to should be drawn as soon as it is known rather than
                // waiting on a body that may still be fetching.
                //
                // `fill_reader` below has always used `loaded.envelope` to
                // feed the stacked pane's per-entry header. This dropped
                // everything but the body, which is why the one-document
                // pane said nothing about recipients (#1427) -- not because
                // the data was not there.
                if let Some(envelope) = &loaded.envelope {
                    pane.set_thread_recipients(
                        row.id,
                        postio_ui::reader::header::recipient_line(&envelope.to),
                        postio_ui::reader::header::recipient_line(&envelope.cc),
                    );
                }
                if let crate::compose::Body::Ready { body, .. } = loaded.body {
                    pane.set_thread_body(row.id, body);
                }
            });
        }
    }

    fn fill_reader(&self, reader: &postio_gtk::reader::Reader, message: MessageId) {
        let reader = reader.clone();
        let offline_now = self.offline.clone();
        let fetch = self.fetcher();
        self.read_then(message, move |loaded| {
            // Not here yet? Fetch it. The conversation stack builds one
            // of these per message, so this is what makes a thread whose
            // bodies the backfill has not reached fill in as it is read.
            fetch.request_if_partial(message, &loaded);
            // `set_message_header` is still called here, unlike before
            // #487: the conversation entry above already carries
            // sender/subject/date, so the reader's own copies of those
            // stay hidden (`set_identity_visible(false)`, set once when
            // this reader was built) — but recipients have nowhere else
            // to go, and the header is the only place that draws To/Cc.
            if let Some(envelope) = &loaded.envelope {
                reader.set_message_header(
                    &envelope.from,
                    &envelope.to,
                    &envelope.cc,
                    envelope.subject.as_deref(),
                    envelope.date,
                );
            }
            match loaded.body {
                crate::compose::Body::Ready {
                    body,
                    encoding_problems,
                } => {
                    let root = root_type(loaded.content_type.as_deref(), &body, &loaded.parts);
                    reader.set_attachments(&root, &loaded.parts);
                    reader.render(&body, loaded.sender.as_deref());
                    // After `render`, which clears it: the caveat belongs
                    // to this message and must not outlive it (#901).
                    reader.set_encoding_problems(encoding_problems);
                    // Same reason, same convention (#971).
                    reader.set_unsubscribe(loaded.list_identifier.as_deref());
                    // And last, because it is what decides whether that
                    // banner is allowed to stand at all (#1525).
                    reader.set_send_state(loaded.send_state);
                }
                crate::compose::Body::Absent(reason) => {
                    let root = root_type(
                        loaded.content_type.as_deref(),
                        &postio_model::MessageBody::default(),
                        &loaded.parts,
                    );
                    reader.set_attachments(&root, &loaded.parts);
                    reader.show_absent(Self::waiting_reason(&offline_now, reason));
                }
            }
            reader.widget().set_visible(true);
        });
    }

    /// Put `row`'s message in the pane, or say why it cannot be.
    ///
    /// # Why this can be asked twice for one message
    ///
    /// The filler is wired to the cursor *and* to activation, deliberately:
    /// the cursor reports only once the user has moved it, so on a window
    /// nobody has touched the pane is empty and `Enter` still has to open
    /// whatever the autoselect landed on. The overlap was documented as
    /// harmless — "a store read and nothing else" — which was wrong. Every
    /// fill ends in a document handed to WebKit, and every document handed to
    /// WebKit is a teardown, a rebuild, a scroll position lost and a frame of
    /// unpainted `WebView`: #749's black flash, twice for one keystroke.
    ///
    /// So the second ask is skipped when it would change nothing: the pane is
    /// already aimed at this message *and* is currently displaying one.
    /// `window.reading()` is the second half because it is false exactly when
    /// the pane was emptied — a folder change, a message that went away, the
    /// composer taking the pane over — which are the cases where re-asking
    /// for the same message is the right thing to do rather than a repeat.
    fn fill(&self, window: &Window, row: postio_gtk::list::Row) {
        let message = row.id;
        // A row that stands for a conversation gets the conversation pane,
        // not the single reader (#755): ADR 0015 Q4, "The column is an
        // index. The pane is the conversation." A query view's rows are
        // messages — `is_thread` is false there by construction — and a
        // folder row that never got a thread id has no conversation to
        // open, so both fall through to the single-message path below.
        if row.is_thread()
            && let Some(thread) = row.thread
        {
            // The same skip the single path makes below, for the same #749
            // reason: re-opening a conversation the pane already shows
            // re-runs the opening policy, which moves focus out from under
            // whoever has moved it since.
            if self.aimed.get() == Some(message) && window.conversation_on(thread) {
                return;
            }
            self.aimed.set(Some(message));
            window.open_conversation(&row);
            // `showing` — what `e` replies to — is not set here: it follows
            // the pane's focus through `connect_focus_changed` above, and
            // the pane's opening policy has a better answer than the row's
            // representative.
            return;
        }
        if self.aimed.get() == Some(message) && window.reading() {
            return;
        }
        self.aimed.set(Some(message));
        self.showing.set(Some(message));

        let painter = self.painter(window);
        let fetch = self.fetcher();
        self.read_then(message, move |loaded| {
            if !painter.still_showing(message) {
                return;
            }
            // After the guard, so only the message the pane settled on
            // has its body fetched -- a held-down `j` that swept past
            // this one asks for nothing.
            fetch.request_if_partial(message, &loaded);
            painter.paint(message, loaded);
        });
    }

    /// A body or a payload for `message` is now on this machine (#396,
    /// #739).
    ///
    /// Two things decide whether this repaints anything, for each of the two
    /// panes that can be showing `message` at once — the single reading pane
    /// and, independently, one entry of the conversation pane (#308).
    ///
    /// **Who it is for.** The engine emits [`Event::BodyLoaded`] for every
    /// body it commits, and a backfill commits thousands the user is not
    /// looking at. Only an arrival for a message actually on screen changes
    /// anything, so that is the whole of the guard — and it is checked here,
    /// before a read is even queued, rather than after one. The single pane
    /// asks `showing`; the conversation pane asks whether it has this
    /// message expanded, which is a different question — several of its
    /// entries can be expanded at once, none of them need be `showing`
    /// (that cell aims the reply verbs at whichever is *focused*), and an
    /// arrival can be for one that is collapsed, which repaints nothing.
    ///
    /// **How often.** A backfill emits these in bursts, so each repaint is
    /// coalesced onto the next turn of the main loop: twenty arrivals for the
    /// same message are one store read and one repaint, not twenty of each.
    /// `Folders::reload` coalesces a resync's `MessagesChanged` the same way
    /// and for the same reason. The conversation used to coalesce *per
    /// message*, because a burst could carry arrivals for several expanded
    /// entries and each was its own pane to redraw; one document has one
    /// pane, and `ConversationView::set_thread_body` does that coalescing
    /// now (#1426).
    ///
    /// [`Event::BodyLoaded`]: postio_core::Event::BodyLoaded
    fn body_arrived(self: &Rc<Self>, window: &Window, message: MessageId) {
        if self.showing.get() == Some(message) && !self.queued.replace(true) {
            let parts = Rc::clone(self);
            let window = window.downgrade();
            glib::idle_add_local_once(move || {
                parts.queued.set(false);
                let Some(window) = window.upgrade() else {
                    return;
                };
                parts.repaint(&window);
            });
        }

        // And the one-document conversation pane, when the body is for a
        // message it is showing. One `WebView` for the whole thread (ADR
        // 0032), so this reads the body and hands it to
        // `set_thread_body`, which coalesces arrivals into one redraw.
        //
        // Without this a conversation whose bodies the backfill had not
        // reached stayed a stack of empty headers until it was closed and
        // reopened -- `fill_thread` requested the bodies but nothing drew
        // them when they landed. Guarded by `rows()` so a body for a thread
        // that is not open is not inserted into the pane's map.
        let conversation = window.conversation();
        if conversation.rows().iter().any(|row| row.id == message) {
            self.read_then(message, move |loaded| {
                if let crate::compose::Body::Ready { body, .. } = loaded.body {
                    conversation.set_thread_body(message, body);
                }
            });
        }
    }

    /// Read whatever the pane is showing again and draw it.
    ///
    /// No `Row` and no cursor movement: this is the same message it was
    /// already showing, with more of it local than there was.
    fn repaint(&self, window: &Window) {
        let Some(message) = self.showing.get() else {
            return;
        };
        let painter = self.painter(window);
        self.read_then(message, move |loaded| {
            // The cursor can still have moved between queueing this and
            // the store answering — the same race `fill` guards, reached
            // by a different road.
            if !painter.still_showing(message) {
                return;
            }
            painter.paint(message, loaded);
            painter.refresh_parts();
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

/// Everything the reading pane needs about one message, as [`Fill::read`]
/// hands it back across the channel.
struct Loaded {
    /// The words, or which kind of "no body" this is.
    body: crate::compose::Body,
    /// The message's own content type — the parts tree's root row.
    content_type: Option<String>,
    /// Its parts, as `BODYSTRUCTURE` described them. Bytes not included.
    parts: Vec<Attachment>,
    /// The header fields, once they have synced (#319).
    envelope: Option<Envelope>,
    /// The allow-list key for remote images: the sender's address.
    ///
    /// Read from the message row rather than from the list row it was opened
    /// through, because a repaint has no list row — and one answer that all
    /// three callers share cannot disagree with itself.
    sender: Option<String>,
    /// Whether this message is one being sent, and in which state (#1525).
    ///
    /// `None` for ordinary mail. It decides which verbs the reading pane
    /// offers, because Reply, Forward and Archive are all answers to
    /// somebody else's mail.
    send_state: Option<postio_model::DraftState>,
    /// The unsubscribe banner's list, per #971: `List-Id` when the message
    /// has one, the sender's domain otherwise. `None` only when the message
    /// itself is gone, since every message has at least one of the two.
    list_identifier: Option<String>,
}

/// What [`postio_gtk::reader::Reader::set_unsubscribe`] shows for `message`,
/// per #971's own doc comment: the `List-Id` header when there is one, the
/// sender's domain otherwise.
///
/// The rule moved to `postio_ui::unsubscribe` (#1585), where the macOS reader
/// can reach it — it decides which list an activation gets recorded against,
/// which is not a thing two frontends may answer separately. This is the
/// shape the store hands over, and nothing else.
fn list_identifier(message: &Message) -> Option<String> {
    postio_ui::unsubscribe::list_identifier(message.list_id.as_deref(), &message.from)
}

/// Draw `loaded` into the window's single reading pane.
///
/// Free rather than a method on [`Fill`]: every caller is an `async` block
/// that has already crossed to the store and back, and holding a borrow of
/// `Fill` across that await is exactly the shape of the reentrancy this
/// module cannot afford. What it needs is the four things it writes.
fn paint(
    window: &Window,
    opened: &RefCell<Option<Opened>>,
    named_accounts: &[(postio_model::AccountId, String)],
    offline: &Cell<bool>,
    message: MessageId,
    loaded: Loaded,
) {
    // Whether the pane already has this exact document up (#749).
    //
    // The header and the chips below are redrawn regardless: they are cheap,
    // and a payload landing genuinely changes a chip. What is skipped is the
    // document — which is a full WebKit teardown and reload, a frame of
    // unpainted view, and the reader's scroll position discarded. A backfill
    // emits `BodyLoaded` for every payload it commits, so for a message
    // someone is reading that was happening repeatedly, and each time it
    // yanked them back to the top of a body they were partway down.
    let signature = (
        message,
        document_signature(&loaded.body, loaded.sender.as_deref(), offline.get()),
    );
    //
    // `window.reading()` is half the question, and not a formality: the pane
    // can be emptied without `opened` being touched — a folder change, a
    // message that went away, the composer taking the pane over — and after
    // that the last signature describes a document that is no longer on
    // screen. Trusting it alone would leave the pane blank for a message the
    // user had just clicked, which is #70 wearing a new hat.
    let already_showing = window.reading()
        && opened
            .borrow()
            .as_ref()
            .is_some_and(|open| open.signature == signature);
    // The envelope is known as soon as headers have synced -- well before a
    // body necessarily is -- so the header goes on screen regardless of which
    // arm below the body takes (#319).
    if let Some(envelope) = &loaded.envelope {
        window.reader().set_message_header(
            &envelope.from,
            &envelope.to,
            &envelope.cc,
            envelope.subject.as_deref(),
            envelope.date,
        );
        // Whose mail this is. Silent with one account, because
        // `named_accounts` is empty then and there is nothing to say (#185).
        let named = named_accounts
            .iter()
            .position(|(id, _)| *id == envelope.account)
            .map(|hue| (hue, named_accounts[hue].1.as_str()));
        window
            .reader()
            .set_account(named.map(|(_, name)| name), named.map_or(0, |(h, _)| h));
    }
    match loaded.body {
        crate::compose::Body::Ready {
            body,
            encoding_problems,
        } => {
            let root = root_type(loaded.content_type.as_deref(), &body, &loaded.parts);
            window.reader().set_attachments(&root, &loaded.parts);
            *opened.borrow_mut() = Some(Opened {
                root,
                parts: loaded.parts,
                absent: None,
                signature,
            });
            if !already_showing {
                window.show_message(&body, loaded.sender.as_deref());
            }
            // Outside the `already_showing` guard on purpose. That guard is
            // about not repainting a document that has not changed, and this
            // is not part of the document -- a repaint suppressed there would
            // otherwise leave the caveat off a message that needs it.
            window.reader().set_encoding_problems(encoding_problems);
            window
                .reader()
                .set_unsubscribe(loaded.list_identifier.as_deref());
            // Outside that guard for the same reason, and after the banner
            // it can withdraw (#1525).
            window.reader().set_send_state(loaded.send_state);
        }
        crate::compose::Body::Absent(reason) => {
            // The chips still go on. They are drawn from `BODYSTRUCTURE`
            // metadata the sync already stored, so a message nothing has been
            // fetched for can still say what came with it -- which is worth
            // more than a blank pane, and is the one part of this state that
            // is not a wait.
            let root = root_type(
                loaded.content_type.as_deref(),
                &postio_model::MessageBody::default(),
                &loaded.parts,
            );
            let reason = Fill::waiting_reason(offline, reason);
            window.reader().set_attachments(&root, &loaded.parts);
            *opened.borrow_mut() = Some(Opened {
                root,
                parts: loaded.parts,
                absent: Some(reason),
                signature,
            });
            if !already_showing {
                window.show_absent(reason);
            }
        }
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
    /// Which message the pane is drawing, and a digest of exactly what
    /// `Reader::render` was given for it -- see [`document_signature`].
    signature: (MessageId, u64),
}

/// A digest of everything the reading pane turns into a document: the body it
/// would render and the sender the remote-image policy is keyed on, or the
/// wait it would explain instead.
///
/// Paired with a [`MessageId`] by its caller, because bytes alone are not
/// identity here. Two different messages can compose the identical document —
/// `gtk_reader_scroll` renders one body under two senders precisely to check
/// that opening the second still starts at the top — so a comparison that
/// looked only at the document would leave the reader scrolled halfway down a
/// message the user had just left. The message is what makes "the same thing
/// is already on screen" true; the digest is what makes it *still* true.
/// `offline` is part of it because it is part of what gets drawn: the same
/// stored reason renders as "waiting on the network" or "you are offline"
/// depending on it (see [`Fill::waiting_reason`]), so leaving it out would let
/// a connectivity change be mistaken for nothing having changed.
fn document_signature(body: &crate::compose::Body, sender: Option<&str>, offline: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    offline.hash(&mut hasher);
    match body {
        crate::compose::Body::Ready {
            body,
            encoding_problems,
        } => {
            0u8.hash(&mut hasher);
            body.text.hash(&mut hasher);
            body.html.hash(&mut hasher);
            // Part of the signature, so a body that gained or lost the caveat
            // counts as a different document. Without it a reparse that
            // changed only this would be suppressed as "already showing".
            encoding_problems.hash(&mut hasher);
        }
        crate::compose::Body::Absent(reason) => {
            1u8.hash(&mut hasher);
            format!("{reason:?}").hash(&mut hasher);
        }
    }
    sender.hash(&mut hasher);
    hasher.finish()
}

/// The header fields the reading pane needs (#319), pulled out of a full
/// [`Message`] row so `Fill::fill`'s database closure hands only what the
/// GTK side needs across the channel, not the whole row.
struct Envelope {
    /// Which account it arrived in. Read here rather than looked up later
    /// because the message row is already in hand and the reading pane is
    /// where #185 answers "whose is this?".
    account: postio_model::AccountId,
    from: Vec<EmailAddress>,
    to: Vec<EmailAddress>,
    cc: Vec<EmailAddress>,
    subject: Option<String>,
    /// The sender's own `Date`, falling back to when the server received it
    /// -- always known -- for the rare message with no `Date` header at all.
    date: chrono::DateTime<chrono::Utc>,
}

impl From<Message> for Envelope {
    fn from(message: Message) -> Self {
        Self {
            account: message.account_id,
            from: message.from,
            to: message.to,
            cc: message.cc,
            subject: message.subject,
            date: message.date.unwrap_or(message.received_at),
        }
    }
}

/// How long a save waits for a body it had to ask for.
///
/// Long enough for a slow server on a bad link, short enough that a save that
/// is never going to work says so while the user is still looking at it.
const BODY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

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
    database: Store,
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
///
/// # Every part gets a name of its own
///
/// The names come from [`postio_gtk::parts::save_names`], over the whole set
/// at once, and not from asking each node what it is called. Nothing stops a
/// message carrying two parts that both say `invoice.pdf`, and naming them
/// one at a time writes the second over the first: a directory with one
/// invoice in it, no error, and no sign that a second ever arrived. That is a
/// silent loss of the user's mail from the one command whose whole promise is
/// that it got everything.
///
/// The rule is shared with the macOS boundary rather than written twice —
/// `postio_session::reading::save_all_parts` resolves the same collision from
/// the same function — so a repeat lands as `invoice-2.pdf` on both
/// frontends, compared without case because the filesystem under one of them
/// is.
pub(crate) async fn save_all_parts(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    into: &std::path::Path,
    message: MessageId,
    nodes: &[postio_gtk::parts::Node],
) -> usize {
    let names = postio_gtk::parts::save_names(nodes);
    let mut failed = 0;
    for (node, name) in nodes.iter().zip(&names) {
        if crate::export::export_part_as(database, blobs, engine.clone(), into, message, node, name)
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
    // POSTIO-CONSENT: runs only from the parts panel's own Open / Open with…
    // commands — a per-part, deliberate activation on a file already saved
    // locally. What the desktop's handler then does is the user's choice of
    // application; Postio opens no connection here and nothing runs on
    // render.
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
pub(crate) async fn wait_for_body(database: &Store, message: MessageId) -> Result<BlobId, String> {
    let deadline = std::time::Instant::now() + BODY_WAIT;
    loop {
        // A read that fails here is usually the writer we are waiting for
        // holding the table, so contention is a reason to look again rather
        // than to give up. Only the deadline ends this.
        match raw_blob(database, message).await {
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

/// Just the raw-message blob key. What the wait watches for.
pub(crate) async fn raw_blob(
    database: &Store,
    message: MessageId,
) -> Result<Option<BlobId>, String> {
    Ok(read_message(database, message).await?.raw_blob_id)
}

pub(crate) async fn read_message(
    database: &Store,
    message: MessageId,
) -> Result<postio_model::Message, String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".into())
}

// `cid_source` moved to `postio_session::reading` (#608). What a `Content-ID`
// may resolve to is a security property both frontends have to agree on, not
// a fact about this one.
pub(crate) use postio_session::reading::cid_source;

// `part_bytes` went the same way, for the same reason and with more at stake
// (#1572). Getting one part's bytes is not glue: it is ADR 0017's payload
// axis, the `AttachmentId` that does not survive a whole-message refetch, and
// the #109 race between an open and the backfill chasing the same message.
// None of that is about GTK, and the macOS boundary needs every line of it --
// so there is one implementation and this crate calls it, rather than two
// that would have reproduced the bugs instead of the behaviour.
//
// `PartSource`, `locate_part`, `part_path` and `wait_for_part` went with it:
// they were only ever how this worked.
pub(crate) use postio_session::reading::part_bytes;

// `root_type` is `postio_ui::reader::parts`' now. A message's own content
// type is what the parts tree hangs off, and the macOS panel hangs its tree
// off the same answer.
use postio_ui::reader::parts::root_type;

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

    use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
    use postio_model::MailboxRole;
    use postio_runtime::engine::{EngineParts, NetworkSource, SystemClock};
    use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
    use postio_storage::seed::seed_small;
    use postio_storage::test_support::TempStore;
    use postio_storage::{BlobStore, Store, test_support};

    use super::*;

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
    async fn world() -> (TempStore, BlobStore, Engine, MessageId, tempfile::TempDir) {
        let database = test_support::temp().await;
        let report = seed_small(&database, 11).await;
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (sink, _events) = postio_core::bridge::event_channel();

        let mut mailbox = MockMailbox::new(&inbox.path);
        for n in 1..=40 {
            // multipart/mixed, so part 2 is a real attachment the parser
            // will hand back with its own decoded bytes.
            mailbox = mailbox.message(
                MockMessage::new(
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
                )
                // So a `BODY.PEEK[2]` has something to answer with. The mock
                // has no MIME parser and rejects a section nobody seeded, so
                // without this a payload fetch fails rather than quietly
                // costing nothing.
                .with_part("2", ATTACHED.as_bytes()),
            );
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
        let connection = database.connect().await.expect("a connection");
        connection
            .execute(
                "UPDATE messages SET uid = id + 1000, uid_validity = 1,
                        remote_id = '1:' || (id + 1000)
                  WHERE mailbox_id = ?1",
                [inbox.id.get()],
            )
            .await
            .expect("the fixture writes");
        let newest = MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: 1,
                after: None,
            })
            .await
            .expect("a page")
            .first()
            .expect("the inbox has mail")
            .id;
        connection
            .execute(
                "UPDATE messages SET uid = 1, uid_validity = 1, remote_id = '1:1' WHERE id = ?1",
                [newest.get()],
            )
            .await
            .expect("the fixture writes");
        drop(connection);

        let engine = Engine::spawn(EngineParts {
            account: report.account.id,
            database: (*database).clone(),
            blobs: blobs.clone(),
            backend: Arc::new(MockBackend::builder().mailbox(mailbox).build()),
            // Never dialled: nothing here queues a send.
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
                postio_account::secret::MemorySecretStore::default(),
            ))),
            events: sink,
            retry: Default::default(),
            backfill: Default::default(),
            reconnect: Default::default(),
            watch: Default::default(),
            network: NetworkSource::default(),
            mailbox_roles: Default::default(),
            clock: Arc::new(SystemClock),
        })
        .expect("an engine");

        (database, blobs, engine, newest, directory)
    }

    /// As [`a_part_not_here`], but with the MIME headers `BODYSTRUCTURE` would
    /// have recorded — so the part can be fetched by section rather than by
    /// dragging the whole message across.
    ///
    /// The distinction is ADR 0017's payload axis: a row that has these takes
    /// one `BODY.PEEK[2]`, and a row that does not falls back to every byte,
    /// because a fetched section arrives encoded with nothing to say how.
    async fn a_part_fetchable_by_section(database: &Store, message: MessageId) -> AttachmentId {
        a_part_not_here(database, message).await;
        let connection = database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut row = messages
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        row.attachments[0].part_headers = Some("Content-Type: application/pdf\r\n".to_owned());
        // The row id changes under this: `update` replaces a message's
        // attachment rows rather than editing them, which is the very reason
        // `part_bytes` resolves an id to a MIME path before it fetches
        // anything. So the id is read back after the write, not before.
        update_with_retry(&messages, &mut row).await;
        messages
            .get(message)
            .await
            .expect("a read")
            .expect("the message")
            .attachments
            .first()
            .expect("the part was written")
            .id
    }

    /// The attachment row for the message the mock will serve, and its id.
    ///
    /// The store's row and the server's message have to describe the same
    /// part, which the seed cannot arrange on its own: it fills a screenshot
    /// from the corpus and knows nothing about any server.
    async fn a_part_not_here(database: &Store, message: MessageId) -> AttachmentId {
        let connection = database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut row = messages
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        assert!(
            row.raw_blob_id.is_none(),
            "the fixture already has this message's bytes, so this proves nothing"
        );

        let mut part = postio_model::Attachment::new(message, "application/pdf", 9);
        part.filename = Some("report.pdf".to_owned());
        // The MIME path the mock's message puts the attachment at.
        part.part_id = Some("2".to_owned());
        row.attachments = vec![part];
        update_with_retry(&messages, &mut row).await;

        messages
            .get(message)
            .await
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
    async fn update_with_retry(messages: &MessageRepository<'_>, row: &mut postio_model::Message) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match messages.update(row).await {
                Ok(()) => return,
                Err(error) if std::time::Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("the fixture writes: {error}"),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_part_nobody_has_is_fetched_before_it_is_saved() {
        // postio-v62's last criterion, arranged so it cannot pass without the
        // fetch: `part_bytes` is the only thing here that talks to the engine,
        // and the message has no raw blob until it does. A version of this
        // that called `request_body` itself first would prove only that bytes
        // already on disk can be read, which was never in doubt.
        let (database, blobs, engine, message, _directory) = world().await;
        let part = a_part_not_here(&database, message).await;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_part_with_no_engine_says_so_rather_than_saving_nothing() {
        // The account is not syncing. Writing an empty file would look like a
        // successful save and would not be one.
        let (database, blobs, _engine, message, _directory) = world().await;

        let part = a_part_not_here(&database, message).await;

        let refused = part_bytes(&database, &blobs, None, message, part)
            .await
            .expect_err("there is no engine to fetch with");

        assert!(
            refused.contains("not syncing"),
            "the sentence has to say why: {refused}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_all_parts_fetches_what_it_needs_for_every_leaf() {
        // Same shape as `a_part_nobody_has_is_fetched_before_it_is_saved`, but
        // through the `S` path: nothing here is downloaded yet, so `S` must
        // fetch before it writes.
        let (database, blobs, engine, message, _directory) = world().await;
        let attachment = a_part_not_here(&database, message).await;
        let node = postio_gtk::parts::Node {
            part_id: "2".to_owned(),
            depth: 1,
            mime: "application/pdf".to_owned(),
            filename: Some("report.pdf".to_owned()),
            size: 9,
            downloaded: false,
            last: true,
            attachment: Some(attachment),
            content_id: None,
            inline: false,
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

    #[tokio::test(flavor = "multi_thread")]
    async fn save_all_parts_counts_a_failure_without_abandoning_the_rest() {
        // A container has no bytes -- `export_part` refuses it -- but the
        // batch must still reach the leaf that comes after it, and the
        // caller has to be told one part did not make it.
        let (database, blobs, engine, message, _directory) = world().await;
        let attachment = a_part_not_here(&database, message).await;
        let container = postio_gtk::parts::Node {
            part_id: String::new(),
            depth: 0,
            mime: "multipart/mixed".to_owned(),
            filename: None,
            size: 0,
            downloaded: true,
            last: false,
            attachment: None,
            content_id: None,
            inline: false,
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
            content_id: None,
            inline: false,
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

    #[tokio::test(flavor = "multi_thread")]
    async fn save_all_parts_does_not_write_one_part_over_another() {
        // Two rows in the panel that both say `report.pdf`, which is an
        // ordinary message rather than a corner: a sender forwarding two
        // statements, or a scanner naming everything after itself. Named one
        // at a time, the second lands on top of the first and `S` reports a
        // clean save of a directory holding half the mail it promised.
        //
        // The collision is resolved by `postio_gtk::parts::save_names` over
        // the whole set, which is `postio_ui`'s function and the same one the
        // macOS boundary uses -- so what this is really asserting is that
        // this side calls it at all.
        let (database, blobs, engine, message, _directory) = world().await;
        let attachment = a_part_not_here(&database, message).await;
        let node = postio_gtk::parts::Node {
            part_id: "2".to_owned(),
            depth: 1,
            mime: "application/pdf".to_owned(),
            filename: Some("report.pdf".to_owned()),
            size: 9,
            downloaded: false,
            last: false,
            attachment: Some(attachment),
            content_id: None,
            inline: false,
        };
        let into = tempfile::tempdir().expect("a save directory");

        let failed = save_all_parts(
            &database,
            &blobs,
            Some(engine),
            into.path(),
            message,
            &[node.clone(), node],
        )
        .await;

        assert_eq!(failed, 0, "both parts had bytes to save");
        let written = std::fs::read_dir(into.path())
            .expect("the save directory")
            .count();
        assert_eq!(
            written, 2,
            "two parts claiming one name overwrote each other: `S` promised \
             everything and wrote {written} file(s)"
        );
    }

    // -----------------------------------------------------------------------
    // The payload axis (ADR 0017, #377)
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn opening_a_part_fetches_that_section_and_nothing_around_it() {
        // The column the receive path never wrote. The message here is
        // `multipart/mixed` with a forty-byte payload, but the shape is the
        // one that matters: on the reference account the same fetch used to
        // drag the whole message, and ~90% of a mailbox by weight is
        // attachments FTS5 cannot index.
        let (database, blobs, engine, message, _directory) = world().await;
        let part = a_part_fetchable_by_section(&database, message).await;

        let bytes = part_bytes(&database, &blobs, Some(engine), message, part)
            .await
            .expect("the part is fetched and handed back");

        assert_eq!(String::from_utf8_lossy(&bytes).trim(), ATTACHED);

        let connection = database.connect().await.expect("a connection");
        let row = MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
            .expect("the message");
        assert!(
            row.attachments[0].is_downloaded(),
            "the bytes have to be recorded against the part, or the chip              still cannot tell 'download' from 'open'"
        );
        assert!(
            row.raw_blob_id.is_none(),
            "and the message around the part was never pulled"
        );
    }

    /// The attachment row currently carrying `part_id`, resolved fresh.
    ///
    /// What a repainted panel holds: `MessageRepository::update` replaces a
    /// message's attachment rows wholesale, so an `AttachmentId` resolved
    /// before any concurrent update — the background lane finishing a fetch,
    /// say — names a row that no longer exists. The MIME path is the name
    /// that survives, which is the same reason `part_bytes` converts to it
    /// first thing.
    async fn the_part_as_stored(
        database: &Store,
        message: MessageId,
        part_id: &str,
    ) -> AttachmentId {
        let connection = database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
            .expect("the message")
            .attachments
            .iter()
            .find(|part| part.part_id.as_deref() == Some(part_id))
            .expect("the part is still in the message")
            .id
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_part_already_on_this_machine_is_read_without_an_engine_at_all() {
        // The second open. Passing `None` for the engine is the strongest
        // form of "no network fetch" this seam can state: any path that
        // reached for the server would refuse instead of answering.
        let (database, blobs, engine, message, _directory) = world().await;
        let part = a_part_fetchable_by_section(&database, message).await;

        part_bytes(&database, &blobs, Some(engine), message, part)
            .await
            .expect("the first open fetches it");

        // Resolved again, not reused. `MessageRepository::update` REPLACES a
        // message's attachment rows — part_bytes's own doc is built on it —
        // and `world()`'s engine backfills this very message in the
        // background (ADR 0016), so the id resolved before the first open is
        // dead by now whenever that fetch won the race: 8 of 180 hammered
        // runs failed here holding the old id (#109). The panel a person
        // clicks re-reads the row on the store's events, so resolving from
        // the store as it is *now* is what the second open actually does.
        let part = the_part_as_stored(&database, message, "2").await;
        let bytes = part_bytes(&database, &blobs, None, message, part)
            .await
            .expect("the second open must not need a server");

        assert_eq!(String::from_utf8_lossy(&bytes).trim(), ATTACHED);
    }
}
