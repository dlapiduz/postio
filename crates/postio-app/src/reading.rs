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

use postio_gtk::reader::RemoteImageAllowList;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_core::ConnectionState;
use postio_core::bridge::EventSink;
use postio_gtk::feed::Feeds;
use postio_gtk::reader::Absent;
use postio_gtk::sidebar::SyncStatus;
use postio_gtk::window::Window;
use postio_model::address::EmailAddress;
use postio_model::{Attachment, Message, MessageId};

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
async fn accounts_to_name(client: &Client) -> Vec<(postio_model::AccountId, String)> {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let accounts = client.accounts().await.unwrap_or_default();
    if accounts.len() < 2 {
        return Vec::new();
    }
    accounts
        .into_iter()
        .map(|account| (account.id, account.display_name))
        .collect()
}

pub async fn install(
    window: &Window,
    wiring: &Wiring,
    client: Client,
    feeds: &Feeds,
    showing: Showing,
) {
    // See `accounts_to_name`: empty in the single-account case, which is the
    // common one, and then this costs a length check per message.
    let named_accounts: Rc<Vec<(postio_model::AccountId, String)>> =
        Rc::new(accounts_to_name(&client).await);
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

    // #971: the banner only asks — logging the activation is the store's
    // owner's, which `postio-gtk` may not reach. `showing` is read at click
    // time rather than captured with the message, because the two can only
    // ever agree: the banner names whichever message is currently on screen.
    // The host names the list by the banner's own rule (`List-Id`, else the
    // sender's domain), so the banner's words are not sent back.
    window.reader().connect_unsubscribe_activated({
        let client = client.clone();
        let runtime = wiring.runtime.clone();
        let showing = showing.clone();
        move |_list_identifier| {
            let Some(message) = showing.get() else {
                return;
            };
            let client = client.clone();
            runtime.spawn(async move {
                if let Err(error) = client.unsubscribe(message).await {
                    tracing::warn!(%error, "could not record an unsubscribe");
                }
            });
        }
    });

    window.set_blob_source(cid_source(
        {
            let showing = showing.clone();
            move || showing.get()
        },
        client.clone(),
    ));

    // `s` in the parts panel. The panel has already run the portal dialog and
    // chosen the file; everything left is bytes, which is this crate's half.
    window.parts().connect_save(glib::clone!(
        #[strong]
        showing,
        #[strong]
        client,
        #[strong(rename_to = events)]
        wiring.events,
        #[strong(rename_to = runtime)]
        wiring.runtime,
        move |node, file| {
            let (Some(attachment), Some(message)) = (node.attachment, showing.get()) else {
                return;
            };
            // The host writes the bytes, so the place has to be one it can
            // name; the portal hands back a local path for every choice.
            let Some(to) = file.path() else {
                events.emit(postio_core::Event::Error {
                    message: "Could not save that part: that place is not a file on this \
                              computer."
                        .to_owned(),
                });
                return;
            };
            let (client, events, runtime) = (client.clone(), events.clone(), runtime.clone());
            glib::spawn_future_local(async move {
                // Runtime work, not main-context work: a part that has not
                // been downloaded is fetched and waited for on a
                // `tokio::time::sleep`, which panicked here with "there is no
                // reactor running" -- the same fault as postio-66. So it goes
                // over to the runtime and answers on a channel, like every
                // other crossing.
                let (sender, receiver) = async_channel::bounded(1);
                runtime.spawn(async move {
                    let saved = client.save_part(message, attachment, to).await;
                    let _ = sender.send(saved.map_err(|error| error.to_string())).await;
                });
                let outcome = match receiver.recv().await {
                    Ok(Ok(_)) => Ok(()),
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
        client: client.clone(),
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
        move |node| {
            let always_ask = !postio_gtk::parts::previewable(&node.mime);
            opener.open_externally(&window, showing.get(), node, always_ask);
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
        move |node| {
            opener.open_externally(&window, showing.get(), node, true);
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
        #[strong]
        client,
        #[strong(rename_to = events)]
        wiring.events,
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
            let (client, events, runtime) = (client.clone(), events.clone(), runtime.clone());
            glib::spawn_future_local(async move {
                // `save_all_parts` is runtime work for the same reason a
                // single save is: a part not yet downloaded waits on
                // `tokio::time::sleep`, which panics off the runtime.
                let (sender, receiver) = async_channel::bounded(1);
                runtime.spawn(async move {
                    let failed = save_all_parts(&client, &into, message, &leaves).await;
                    let _ = sender.send(failed).await;
                });
                // Every part failed is the safe fallback if the runtime
                // vanished mid-batch.
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
        let client = client.clone();
        let runtime = wiring.runtime.clone();
        std::rc::Rc::new(move |node: postio_gtk::parts::Node| {
            let (client, runtime) = (client.clone(), runtime.clone());
            let message = showing.get();
            Box::pin(async move {
                let message = message.ok_or("There is no message open to take a part from")?;
                let into = crate::paths::export_dir();
                // On the runtime: this may wait on a fetch and writes a file.
                // None of that belongs on the UI thread, and the drop is
                // already asynchronous to GTK.
                let (send, receive) = async_channel::bounded(1);
                runtime.spawn(async move {
                    let outcome = crate::export::export_part(&client, &into, message, &node).await;
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
        client,
        runtime,
        showing,
        opened,
        named_accounts,
        offline: Rc::new(Cell::new(is_offline(&feeds.folders.status()))),
        queued: Cell::new(false),
        aimed: Cell::new(None),
        ahead: RefCell::new(None),
        paints: Rc::new(Cell::new(0)),
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
                // While this one is read, the next one is prepared.
                fill.prepare_next(&window);
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
    client: Client,
}

impl BodyFetcher {
    /// Ask the store's owner to fetch `message`'s body if `loaded` says it
    /// is not here.
    ///
    /// [`Absent::Partial`] is "headers synced, body not fetched"; the
    /// backfill reaches it eventually, and this is what makes opening it now
    /// jump the queue. The engines are the host's, so the ask is a posted
    /// request: nothing here waits on it, and a store nobody is syncing
    /// simply fetches nothing. Any other absence no fetch would change.
    fn request_if_partial(&self, message: MessageId, loaded: &Loaded) {
        if let crate::compose::Body::Absent(Absent::Partial) = &loaded.body {
            self.client.fetch_body(message);
        }
    }
}

/// What draws the single reading pane, for a closure that has outlived the
/// [`Fill`] it came from — see [`Fill::painter`]: the cells [`paint`] reads,
/// and the window it paints into.
#[derive(Clone)]
struct Painter {
    window: Window,
    showing: Showing,
    opened: Rc<RefCell<Option<Opened>>>,
    named_accounts: Rc<Vec<(postio_model::AccountId, String)>>,
    offline: Rc<Cell<bool>>,
    /// Which paint is the latest, so a waiting plate held back by
    /// [`PLATE_DELAY`] stands down when anything paints after it.
    paints: Rc<Cell<u64>>,
}

/// How long a message with no body leaves the pane as it was before saying
/// so.
///
/// Drawing the plate is a full document load, and on most cursor moves in a
/// mailbox still backfilling the body it explains arrives a moment later --
/// so the pane flashed Postio's own "waiting" words for a wait nobody had
/// time to see. Under this, the previous message stays put and the body is
/// drawn straight over it; past it, the wait is long enough to be worth
/// explaining. Below what a person notices as a change and well above a
/// local fetch.
const PLATE_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

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
    ///
    /// A message with no body is held back for [`PLATE_DELAY`] unless the
    /// pane is already on it: the header, chips and plate all wait, so the
    /// pane changes once -- to the body, if it lands in time -- rather than
    /// to the plate and then again.
    fn paint(&self, message: MessageId, loaded: Loaded) {
        let paint = self.paints.get().wrapping_add(1);
        self.paints.set(paint);
        let absent = matches!(loaded.body, crate::compose::Body::Absent(_));
        let on_it = self.window.reading()
            && self
                .opened
                .borrow()
                .as_ref()
                .is_some_and(|opened| opened.signature.0 == message);
        if absent && !on_it {
            let painter = self.clone();
            glib::timeout_add_local_once(PLATE_DELAY, move || {
                if painter.paints.get() != paint || !painter.still_showing(message) {
                    return;
                }
                painter.paint_now(message, loaded);
            });
            return;
        }
        self.paint_now(message, loaded);
    }

    /// Draw `loaded` as `message` now — see [`paint`].
    fn paint_now(&self, message: MessageId, loaded: Loaded) {
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

/// How many times a conversation's messages have crossed to the runtime to
/// be read (#1609). Process-wide; for tests.
static THREAD_CROSSINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// See [`THREAD_CROSSINGS`].
#[doc(hidden)]
pub fn thread_crossings() -> u64 {
    THREAD_CROSSINGS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Everything filling the reading pane needs, so the cursor and activation
/// can share one implementation rather than two that drift.
struct Fill {
    /// The store's owner, which every read here goes through (ADR 0041).
    client: Client,
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
    /// The conversation after the one on screen, read and rendered before
    /// anybody opened it -- see [`Fill::prepare_next`].
    ahead: RefCell<Option<Ahead>>,
    /// Which single-pane paint is the latest — see [`Painter::paint`].
    paints: Rc<Cell<u64>>,
}

/// A conversation read and rendered ahead of `j`.
///
/// Only the messages whose bodies were here: one still to come is read when
/// the conversation opens, as before, so preparing never decides what a
/// message *is*, only saves the reading of one that is settled.
struct Ahead {
    thread: postio_model::ids::ThreadId,
    bodies: std::collections::HashMap<MessageId, Loaded>,
    prepared: Vec<postio_ui::reader::document::Prepared>,
}

/// The most messages of a neighbouring conversation prepared ahead: the
/// conversation pane's own first read, so a long thread is not read whole
/// for a keystroke that may never come.
const AHEAD_MESSAGES: u32 = 50;

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
    ///
    /// With `allow`, the body is also judged and sanitised on the runtime
    /// under the policy the allow list gives its sender -- see
    /// [`prepare_loaded`] -- so the pane it is for only loads a document.
    fn read(
        &self,
        message: MessageId,
        allow: Option<(RemoteImageAllowList, Pane)>,
    ) -> async_channel::Receiver<Option<Loaded>> {
        let offline = self.offline.get();
        let client = self.client.clone();
        let (sender, answer) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let loaded = match client.readings(vec![message], offline).await {
                Ok(mut readings) => readings.pop().map(Loaded::from),
                Err(error) => {
                    tracing::warn!(%error, "could not read a message for the reading pane");
                    None
                }
            };
            let _ = sender.send(loaded).await;
        });
        match allow {
            Some((allow, pane)) => {
                crate::search::then_off_thread(&self.runtime, answer, move |loaded| {
                    let scope = match pane {
                        Pane::Single => None,
                        Pane::Conversation => Some(message.get().to_string()),
                    };
                    prepare_loaded(loaded, &allow, scope.as_deref())
                })
            }
            None => answer,
        }
    }

    /// Read every one of `messages` in one call, handing each back in turn
    /// (#1609).
    ///
    /// One crossing to the runtime, and one call to the store's owner, for a
    /// whole conversation, where a crossing per message was a runtime task, a
    /// store turn and a main-context wake-up each.
    ///
    /// With `allow`, each body is also judged and sanitised on the blocking
    /// pool as it comes off the reader, in order, and handed back prepared
    /// under its message's scope -- the conversation document's first draw
    /// used to do both parses for every message on the main thread.
    fn read_each(
        &self,
        messages: Vec<MessageId>,
        allow: Option<RemoteImageAllowList>,
    ) -> async_channel::Receiver<(MessageId, Loaded)> {
        THREAD_CROSSINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let offline = self.offline.get();
        let (sender, receiver) = async_channel::unbounded();
        let sender = match allow {
            Some(allow) => {
                // Read, then prepared: the reader is let go of as soon as the
                // reads are done, and the sanitising never holds it.
                let (read, prepare) = async_channel::unbounded::<(MessageId, Loaded)>();
                let allow = std::sync::Arc::new(allow);
                self.runtime.spawn(async move {
                    while let Ok((message, loaded)) = prepare.recv().await {
                        let allow = std::sync::Arc::clone(&allow);
                        let prepared = tokio::task::spawn_blocking(move || {
                            prepare_loaded(loaded, &allow, Some(&message.get().to_string()))
                        })
                        .await;
                        let Ok(loaded) = prepared else {
                            return;
                        };
                        if sender.send((message, loaded)).await.is_err() {
                            return;
                        }
                    }
                });
                read
            }
            None => sender,
        };
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let readings = match client.readings(messages, offline).await {
                Ok(readings) => readings,
                Err(error) => {
                    tracing::warn!(%error, "could not read a conversation");
                    return;
                }
            };
            for reading in readings {
                let message = reading.message;
                if sender.send((message, Loaded::from(reading))).await.is_err() {
                    // Nobody is listening: the pane moved on.
                    return;
                }
            }
        });
        receiver
    }

    /// Read `message` and, when the answer lands back on the main loop, hand
    /// it to `then`.
    ///
    /// Every fill is this shape — one crossing through [`read`](Self::read),
    /// awaited where the widgets are — and each caller used to spell the
    /// crossing out itself. A read that comes back empty (the message went
    /// away underneath it) is simply not painted.
    fn read_then(
        &self,
        message: MessageId,
        allow: Option<(RemoteImageAllowList, Pane)>,
        then: impl FnOnce(Loaded) + 'static,
    ) {
        // Started before the spawn: `read` borrows `self`, and a `'static`
        // task cannot carry that borrow.
        let answer = self.read(message, allow);
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
            client: self.client.clone(),
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
            paints: Rc::clone(&self.paints),
        }
    }

    /// Read and render the conversation after the cursor's, so `j` onto it
    /// asks the store and the main thread for nothing it already has.
    ///
    /// Bodies are read on one reader turn and let go of before sanitising,
    /// which is CPU and goes to the blocking pool rather than holding a
    /// reader or a runtime worker. Images are prepared blocked, which is
    /// how most senders are drawn; a sender on the allow list is rendered
    /// when shown, as before.
    fn prepare_next(self: &Rc<Self>, window: &Window) {
        let Some(row) = window.list().row_after_cursor() else {
            return;
        };
        let Some(thread) = row.thread.filter(|_| row.is_thread()) else {
            return;
        };
        if self
            .ahead
            .borrow()
            .as_ref()
            .is_some_and(|ahead| ahead.thread == thread)
        {
            return;
        }
        let offline = self.offline.get();
        let client = self.client.clone();
        let (sender, receiver) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let Ok(readings) = client
                .thread_readings(thread, AHEAD_MESSAGES, offline)
                .await
            else {
                return;
            };
            let loaded: Vec<(MessageId, Loaded)> = readings
                .into_iter()
                .map(|reading| (reading.message, Loaded::from(reading)))
                .filter(|(_, answer)| matches!(answer.body, crate::compose::Body::Ready { .. }))
                .collect();
            let ahead = tokio::task::spawn_blocking(move || {
                let prepared = loaded
                    .iter()
                    .filter_map(|(id, answer)| match &answer.body {
                        crate::compose::Body::Ready { body, .. } => {
                            Some(postio_ui::reader::document::prepare(
                                &id.get().to_string(),
                                body,
                                postio_ui::reader::document::RemoteImages::Blocked,
                            ))
                        }
                        _ => None,
                    })
                    .collect();
                Ahead {
                    thread,
                    bodies: loaded.into_iter().collect(),
                    prepared,
                }
            })
            .await;
            if let Ok(ahead) = ahead {
                let _ = sender.send(ahead).await;
            }
        });
        let fill = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads and the
            // rendering run on the runtime above.
            if let Ok(ahead) = receiver.recv().await
                && let Some(fill) = fill.upgrade()
            {
                *fill.ahead.borrow_mut() = Some(ahead);
            }
        });
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
        let thread = rows.first().and_then(|row| row.thread);
        let mut ahead = self
            .ahead
            .borrow_mut()
            .take_if(|ahead| Some(ahead.thread) == thread);
        if let Some(ahead) = ahead.as_mut() {
            // Rendered on a worker; the pane's redraws take these instead of
            // parsing each body again here.
            pane.offer_prepared(std::mem::take(&mut ahead.prepared));
        }
        let mut ready: Vec<(MessageId, Loaded)> = Vec::new();
        let mut ids: Vec<MessageId> = Vec::new();
        for row in &rows {
            match ahead
                .as_mut()
                .and_then(|ahead| ahead.bodies.remove(&row.id))
            {
                Some(loaded) => ready.push((row.id, loaded)),
                None => ids.push(row.id),
            }
        }
        let rows: std::collections::HashMap<MessageId, postio_gtk::list::Row> =
            rows.into_iter().map(|row| (row.id, row)).collect();
        // What was read ahead first, then the store for the rest -- through
        // the one channel, so both are drawn by the same loop below.
        let (early, answers) = async_channel::unbounded();
        for answer in ready {
            let _ = early.send_blocking(answer);
        }
        if !ids.is_empty() {
            let allow = pane
                .document_reader()
                .map(|reader| reader.allowlist_snapshot());
            let rest = self.read_each(ids, allow);
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a channel receive; the reads run on the
                // runtime in `read_each`.
                while let Ok(answer) = rest.recv().await {
                    // Unbounded, so this never waits: it fails only when the
                    // pane has stopped listening.
                    if early.try_send(answer).is_err() {
                        return;
                    }
                }
            });
        } else {
            drop(early);
        }
        let pane = pane.clone();
        let fetch = self.fetcher();
        let named_accounts = Rc::clone(&self.named_accounts);
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads run on the
            // runtime in `read_each`, or ran ahead in `prepare_next`.
            while let Ok((id, mut loaded)) = answers.recv().await {
                let Some(row) = rows.get(&id) else {
                    continue;
                };
                // Sanitised on the runtime; the document's redraw takes it
                // rather than parsing the body again here.
                if let Some(prepared) = loaded.prepared.take() {
                    pane.offer_prepared(vec![prepared]);
                }
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
                //
                // The account too, under the single reader's rule: named
                // only when there is more than one to tell apart (#185), so
                // the pane's header is the reader's header (#1671).
                if let Some(envelope) = &loaded.envelope {
                    let named = named_accounts
                        .iter()
                        .position(|(id, _)| *id == envelope.account)
                        .map(|hue| (named_accounts[hue].1.as_str(), hue));
                    pane.set_thread_envelope(row.id, &envelope.to, &envelope.cc, named);
                }
                match loaded.body {
                    crate::compose::Body::Ready { body, .. } => pane.set_thread_body(row.id, body),
                    // Not on this machine: drawn without it rather than
                    // waited for, and redrawn by `body_arrived` if it lands.
                    _ => pane.set_thread_body_absent(row.id),
                }
            }
        });
    }

    fn fill_reader(&self, reader: &postio_gtk::reader::Reader, message: MessageId) {
        let reader = reader.clone();
        let offline_now = self.offline.clone();
        let fetch = self.fetcher();
        let allow = Some((reader.allowlist_snapshot(), Pane::Single));
        self.read_then(message, allow, move |loaded| {
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
                    reader.render_prepared(&body, loaded.sender.as_deref(), loaded.prepared);
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
        let allow = Some((window.reader().allowlist_snapshot(), Pane::Single));
        self.read_then(message, allow, move |loaded| {
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
            let allow = conversation
                .document_reader()
                .map(|reader| (reader.allowlist_snapshot(), Pane::Conversation));
            self.read_then(message, allow, move |mut loaded| {
                if let Some(prepared) = loaded.prepared.take() {
                    conversation.offer_prepared(vec![prepared]);
                }
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
        let allow = Some((window.reader().allowlist_snapshot(), Pane::Single));
        self.read_then(message, allow, move |loaded| {
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

/// A body as the store's owner said it, in the reader's own words.
fn body_from(body: postio_client::protocol::Body) -> crate::compose::Body {
    use postio_client::protocol::Body as Wire;
    match body {
        Wire::Ready {
            body,
            encoding_problems,
        } => crate::compose::Body::Ready {
            body,
            encoding_problems,
        },
        Wire::Partial => crate::compose::Body::Absent(Absent::Partial),
        Wire::Offline => crate::compose::Body::Absent(Absent::Offline),
        Wire::Missing => crate::compose::Body::Absent(Absent::Missing),
        Wire::Empty => crate::compose::Body::Absent(Absent::Empty),
        Wire::ForeignDraft => crate::compose::Body::Absent(Absent::ForeignDraft),
    }
}

impl From<postio_client::protocol::Reading> for Loaded {
    /// What the pane draws, from what the store's owner read. The parts are
    /// metadata the sync already stored -- `BODYSTRUCTURE`, not bytes.
    fn from(reading: postio_client::protocol::Reading) -> Self {
        let mut row = reading.row.map(|row| *row);
        let sender = row
            .as_ref()
            .and_then(|message| message.from.first().map(|from| from.address.clone()));
        let list_identifier = row.as_ref().and_then(list_identifier);
        let (content_type, parts) = row
            .as_mut()
            .map(|message| {
                (
                    message.content_type.take(),
                    std::mem::take(&mut message.attachments),
                )
            })
            .unwrap_or_default();
        Loaded {
            body: body_from(reading.body),
            content_type,
            parts,
            envelope: row.map(Envelope::from),
            sender,
            send_state: reading.send_state,
            list_identifier,
            prepared: None,
        }
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
    /// The body judged and sanitised on the runtime, when the read was asked
    /// to (`Fill::read`), so the pane only hands WebKit a document.
    prepared: Option<postio_ui::reader::document::Prepared>,
}

/// What [`postio_gtk::reader::Reader::set_unsubscribe`] shows for `message`,
/// per #971's own doc comment: the `List-Id` header when there is one, the
/// sender's domain otherwise.
fn list_identifier(message: &Message) -> Option<String> {
    message.list_id.clone().or_else(|| {
        message
            .from
            .first()
            .and_then(|from| from.domain())
            .map(str::to_owned)
    })
}

/// Which pane a body is being prepared for: they stamp references
/// differently -- see [`prepare_loaded`].
#[derive(Clone, Copy)]
enum Pane {
    /// The single reader, or a stacked conversation's reader for one message.
    Single,
    /// The conversation document, which names each message by its id.
    Conversation,
}

/// Judge and sanitise `loaded`'s body for a reading pane, under the policy
/// `allow` gives its sender.
///
/// `scope` names the message inside a conversation document, whose
/// references are stamped with it; `None` is the single reader. A message
/// with no body is handed back as it was. For the blocking pool: both
/// parses are html5ever over the whole body.
fn prepare_loaded(mut loaded: Loaded, allow: &RemoteImageAllowList, scope: Option<&str>) -> Loaded {
    use postio_ui::reader::document::{RemoteImages, prepare, prepare_message};
    if let crate::compose::Body::Ready { body, .. } = &loaded.body {
        let remote = if loaded
            .sender
            .as_deref()
            .is_some_and(|sender| allow.is_allowed(sender))
        {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        loaded.prepared = Some(match scope {
            Some(scope) => prepare(scope, body, remote),
            None => prepare_message(body, remote),
        });
    }
    loaded
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
                window.show_prepared_message(&body, loaded.sender.as_deref(), loaded.prepared);
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
/// [`Message`] row so what [`Loaded`] keeps is only what the GTK side draws,
/// not the whole row.
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

/// The message's own content type — the row the parts tree hangs off.
///
/// # Read when it is there, derived otherwise
///
/// `BODYSTRUCTURE` says what it is and `postio-account` records it in
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

/// What opening or "Open with…"-ing a part needs, bundled so the seam that
/// actually varies between the two -- `always_ask` -- does not have to travel
/// beside four things that never change per call.
struct PartOpener {
    client: Client,
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
        node: &postio_gtk::parts::Node,
        always_ask: bool,
    ) {
        let Some(message) = message else { return };
        let into = crate::paths::export_dir();
        let (window, node) = (window.clone(), node.clone());
        let (client, events, runtime) = (
            self.client.clone(),
            self.events.clone(),
            self.runtime.clone(),
        );
        glib::spawn_future_local(async move {
            let (sender, receiver) = async_channel::bounded(1);
            runtime.spawn(async move {
                let outcome = crate::export::export_part(&client, &into, message, &node).await;
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
/// toast per failure would be worse than the save. One call to the store's
/// owner for the whole batch; a node with no bytes of its own -- a container
/// -- is counted here without asking. Runtime work, not main-context work: a
/// part not yet downloaded is waited for on `tokio::time::sleep`, which
/// panics off the runtime.
pub(crate) async fn save_all_parts(
    client: &Client,
    into: &std::path::Path,
    message: MessageId,
    nodes: &[postio_gtk::parts::Node],
) -> usize {
    let (targets, refused) = part_targets(into, nodes);
    if targets.is_empty() {
        return refused;
    }
    let asked = targets.len();
    match client.save_parts(message, targets).await {
        Ok(failed) => refused + failed,
        Err(error) => {
            tracing::warn!(%error, "could not save a message's parts");
            refused + asked
        }
    }
}

/// Where each of `nodes` is saved under `into`, and how many have no bytes
/// of their own to save.
fn part_targets(
    into: &std::path::Path,
    nodes: &[postio_gtk::parts::Node],
) -> (
    Vec<(postio_model::ids::AttachmentId, std::path::PathBuf)>,
    usize,
) {
    let mut targets = Vec::new();
    let mut refused = 0;
    for node in nodes {
        match crate::export::part_target(into, node) {
            Ok(target) => targets.push(target),
            Err(_) => refused += 1,
        }
    }
    (targets, refused)
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

// Finding a part's bytes, fetching them if they were never downloaded, and
// waiting for them to land moved to `postio_host::parts`: the store's owner
// does it for every frontend (specs/005-tui-frontend T046), and this pane asks
// it through the client (T018). Re-exported under the names this crate's
// tests always used.
#[cfg(test)]
pub(crate) use postio_host::parts::{part_bytes, read_message, wait_for_body};

/// Resolve a `cid:` reference against the message on screen, through the
/// store's owner.
///
/// What a `Content-ID` may resolve to is a security property every frontend
/// has to agree on (#608), so the host answers it with the one resolution in
/// `postio_session::reading`: scoped to `showing`'s message, never fetched.
///
/// # Why this blocks
///
/// [`BlobSource::resolve`](postio_gtk::reader::BlobSource) is synchronous,
/// because WebKit calls it while laying out a document: the `cid:` URI has to
/// resolve to bytes before the image can be placed, and there is nothing to
/// hand a future to. It was a blocking store read before; it is a blocking
/// call to the host now, the same indexed read on the host's runtime.
pub(crate) fn cid_source(
    showing: impl Fn() -> Option<MessageId> + 'static,
    client: Client,
) -> Rc<dyn postio_ui::reader::parts::BlobSource> {
    Rc::new(move |content_id: &str| {
        let message = showing()?;
        postio_session::blocking::now(client.inline_part(message, content_id.to_owned()))
            .ok()
            .flatten()
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

    use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
    use postio_model::MailboxRole;
    use postio_runtime::engine::{EngineParts, NetworkSource, SystemClock};
    use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
    use postio_storage::seed::seed_small;
    use postio_storage::test_support::TempStore;
    use postio_storage::{BlobStore, Store, test_support};

    use super::*;
    use postio_model::ids::AttachmentId;
    use postio_runtime::Engine;

    /// [`super::save_all_parts`] over a store rather than a client: the same
    /// nodes to paths, and the batch as the host writes it for
    /// `Req::SaveParts` -- so the fetch-before-save below is the host's,
    /// proven without a display.
    async fn save_all_parts(
        database: &Store,
        blobs: &BlobStore,
        engine: Option<Engine>,
        into: &std::path::Path,
        message: MessageId,
        nodes: &[postio_gtk::parts::Node],
    ) -> usize {
        let (targets, refused) = part_targets(into, nodes);
        refused + postio_host::parts::save_parts(database, blobs, engine, message, &targets).await
    }

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
