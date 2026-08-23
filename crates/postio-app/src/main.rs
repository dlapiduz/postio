//! Postio — a local-first, keyboard-first email client.
//!
//! This is the composition root: the one crate that knows both halves of the
//! application exist. It opens the local store, starts the runtime, builds the
//! GTK frontend, and joins them.
//!
//! # Why it is its own crate
//!
//! `postio-gtk` must not depend on `rusqlite` or `io-imap` — the view layer
//! does no SQL and speaks no protocol, and `scripts/check-crate-boundaries.py`
//! enforces it. `postio-gtk` also depends on `postio-core`, and Cargo features
//! are per *package*: if the binary lived in `postio-gtk` and turned on
//! `postio-core/runtime`, feature unification would give the library the same
//! `postio-core`, and `rusqlite` would be back in the view layer's graph.
//!
//! So the composition root has to sit above both. Nothing depends on this
//! crate, so nothing is guarded against what it pulls in — which is exactly
//! the point. Everything below it stays honest.
//!
//! # Startup order
//!
//! The same order `postio_gtk::app::run` documents, because it is not
//! arbitrary: a `PangoContext` keeps the font family it has already resolved,
//! so the embedded faces have to be registered before the first widget exists.
//! What is added here is the last step — opening the store and handing it to
//! the window — which happens on `activate`, after the frontend has built its
//! own.

mod actions;
mod commands;
mod compose;
mod engine;
mod feed;
mod paths;

use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_core::bridge::{Bridge, EventSink, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::startup::{Phase, Timeline};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_runtime::store::{MailStore, SqliteStore};
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, Database};

fn main() -> glib::ExitCode {
    let timeline = Timeline::start();

    if adw::init().is_err() {
        eprintln!("postio: no display; the UI needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    timeline.mark(Phase::Init);

    // Fonts first, before any widget: see the module docs.
    if let Err(error) = fonts::install() {
        // Recoverable: the design degrades to system fallbacks, which is ugly
        // but usable, and refusing to start over a font would be worse.
        eprintln!("postio: {error}");
    }
    timeline.mark(Phase::Fonts);

    if let Some(display) = gdk::Display::default() {
        style::install(&display);
        app::install_icons(&display);
    }
    timeline.mark(Phase::Styles);

    // The store first: the command bus writes it, and the bus has to be built
    // before the runtime that pumps it.
    let store = open_store();

    // What the user is looking at, as the handlers see it. `commands::mirror`
    // brings it into step with the window in the instant before a command is
    // sent; nothing else writes it.
    let state = SharedState::default();
    let bus = match &store {
        Some((database, _)) => {
            actions::dispatcher(actions::Actions::new(database.clone(), state.clone()))
        }
        // No store, so no verb can do anything. An empty bus still answers —
        // every command comes back as "not wired up in this build" — which is
        // a sentence on screen rather than a key that does nothing.
        None => Dispatcher::builder().build(),
    };

    // The runtime: the tokio threads every read is polled on and every command
    // is handled on.
    let (runtime, replies) = match Bridge::new(bus) {
        Ok((bridge, events)) => (Some(bridge), Some(events)),
        Err(error) => {
            eprintln!("postio: no runtime, so no mail: {error}");
            (None, None)
        }
    };

    // The engine's own channel. `Bridge` hands its sink to command handlers
    // and keeps the stream; the engine is not a command handler, so it gets
    // one of its own and the UI drains it on the main context.
    let (sink, events) = event_channel();
    // Taken on the first `activate`. `EventStream` is not `Clone` — there is
    // one queue and exactly one reader of it — and `activate` can fire again
    // when a second launch raises the window.
    let streams = std::cell::RefCell::new(vec![Some(events), replies]);

    let application = app::build_with(timeline);

    // Connected *after* the frontend's own handler, so the window it makes is
    // already there to be fed. Signal handlers run in the order they were
    // connected, which is the whole of the arrangement.
    let wiring = runtime
        .as_ref()
        .zip(store)
        .map(|(bridge, (database, blobs))| Wiring {
            store: Arc::new(SqliteStore::new(&database)),
            database,
            blobs,
            runtime: bridge.handle(),
            events: sink.clone(),
            commands: bridge.commands(),
        });
    application.connect_activate(move |application| {
        let Some(window) = application.active_window().and_downcast::<Window>() else {
            return;
        };
        if let Some(wiring) = &wiring {
            let Some(feeds) = feed_the_window(&window, wiring) else {
                return;
            };
            // Every gesture the window produces from here on reaches a real
            // handler. Before this line the keymap, the palette and the
            // selection model all resolved correctly and then handed off to
            // nothing.
            commands::install(&window, &feeds, state.clone(), wiring.commands.clone());
            // Everything either half has to say reaches the panes here: a
            // mailbox the server disagreed with, a body that arrived, an
            // archive that landed. Two queues because there are two
            // producers — the engine and the bus — and one reader each.
            for stream in streams.borrow_mut().iter_mut().filter_map(Option::take) {
                commands::drain(&window, &feeds, stream);
            }
        }
    });

    let code = application.run();
    if let Some(bridge) = runtime {
        bridge.shutdown();
    }
    code
}

/// What the frontend needs, once there is a store to give it.
struct Wiring {
    database: Database,
    blobs: BlobStore,
    store: Arc<dyn MailStore>,
    runtime: tokio::runtime::Handle,
    events: EventSink,
    commands: postio_core::bridge::CommandSender,
}

/// Open the local store, or explain why there is none.
///
/// A missing or unreadable database is not a reason to refuse to start: the
/// window opens, says it has never synced, and stays usable for everything
/// that does not need mail. A mail client that will not open is worse than one
/// with nothing in it.
fn open_store() -> Option<(Database, BlobStore)> {
    let path = paths::store_path();
    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("postio: cannot open {}: {error}", path.display());
            return None;
        }
    };
    // Beside the database, not inside it: bodies and attachments are
    // content-addressed files, and SQLite holds the key and the metadata.
    let blobs = match BlobStore::open(path.with_file_name("blobs")) {
        Ok(blobs) => blobs,
        Err(error) => {
            eprintln!("postio: cannot open the blob store: {error}");
            return None;
        }
    };
    Some((database, blobs))
}

/// Point the window's panes at the store.
///
/// Silent when there is no account yet: the sidebar already says what is true
/// — offline, never synced, no folders — and inventing an account to fill it
/// would be worse than an empty one. `postio-hiy` is the screen that creates
/// the first one.
fn feed_the_window(window: &Window, wiring: &Wiring) -> Option<postio_gtk::feed::Feeds> {
    let account = first_account(&wiring.database)?;

    // The engine is started before the panes are fed, so that the first
    // thing it does — bring the link up, drain whatever the last session
    // left queued — is already under way while the list is drawing.
    if let Some(sync) = engine::start(
        &account,
        &wiring.database,
        wiring.blobs.clone(),
        wiring.events.clone(),
    ) {
        // Leaked for the same reason the feeds are: it lives as long as the
        // process, and dropping it at exit would stop the engine a moment
        // before the process ends anyway.
        let sync: &'static _ = Box::leak(Box::new(sync));
        seed_the_backfill(sync, wiring);
        fetch_what_is_opened(window, sync, wiring.runtime.clone());
    }

    let sources = feed::Sources::new(wiring.store.clone(), wiring.runtime.clone());
    let feeds = window.install_feeds(
        account.id,
        account.address.address.as_str(),
        sources.clone(),
        sources,
    );

    compose::install(
        window,
        account.id,
        wiring.database.clone(),
        wiring.blobs.clone(),
        wiring.runtime.clone(),
    );

    Some(feeds)
}

/// Jump a message to the front of the backfill when it is opened.
///
/// The one body the user is actually waiting for. Everything else in the
/// queue is a guess about what they will want next; this is not a guess, so
/// it goes to the front of the queue rather than the back.
fn fetch_what_is_opened(
    window: &Window,
    sync: &'static postio_runtime::Engine,
    runtime: tokio::runtime::Handle,
) {
    window.list().connect_activated(move |row| {
        let message = row.id;
        runtime.spawn(async move {
            if let Err(error) = sync.request_body(message).await {
                eprintln!("postio: cannot fetch that message: {error}");
            }
        });
    });
}

/// Ask for the bodies worth having, one mailbox at a time.
///
/// At startup, because a session that ended with mail unread should not have
/// to fetch it again on the wire when the user opens it. `postio-26c` also
/// wants this run again whenever a sync finishes; nothing emits that yet.
fn seed_the_backfill(sync: &'static postio_runtime::Engine, wiring: &Wiring) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let Some(account) = first_account(&wiring.database) else {
        return;
    };
    let Ok(mailboxes) = postio_storage::repository::MailboxRepository::new(&connection)
        .list_for_account(account.id)
    else {
        return;
    };
    drop(connection);

    for mailbox in mailboxes.into_iter().filter(|mailbox| mailbox.selectable) {
        wiring.runtime.spawn(async move {
            if let Err(error) = sync.seed_backfill(mailbox.id, BACKFILL_PER_MAILBOX).await {
                eprintln!("postio: {}: {error}", mailbox.path);
            }
        });
    }
}

/// How many bodies to queue per mailbox at startup.
///
/// A cap rather than the whole folder: the point is that what the user is
/// likely to open next is already local, not that a 40,000-message archive
/// downloads itself on first run.
const BACKFILL_PER_MAILBOX: u32 = 200;

/// The account to open, if the store holds one.
///
/// Read straight off a connection rather than through [`MailStore`]: which
/// account to open is a question about *starting up*, not about drawing mail,
/// and this crate is the one place allowed to ask it directly. It is one
/// indexed read before the window is presented.
fn first_account(database: &Database) -> Option<postio_model::Account> {
    let connection = database
        .connection()
        .map_err(|error| eprintln!("postio: cannot read the accounts: {error}"))
        .ok()?;
    AccountRepository::new(&connection)
        .list_enabled()
        .map_err(|error| eprintln!("postio: cannot read the accounts: {error}"))
        .ok()?
        .into_iter()
        .next()
}
