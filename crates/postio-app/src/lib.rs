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

pub mod actions;
pub mod commands;
pub mod compose;
pub mod engine;
pub mod feed;
pub mod logging;
pub mod paths;
pub mod refresh;
pub mod search;

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

/// Open the store, start the runtime, build the window, and join them.
///
/// The binary is a thin `main` over this. It lives in the library half so
/// that `tests/` can link it: this crate is where the wiring lives, and a
/// wiring nothing can drive is a wiring nothing can check. See the module
/// docs for why that mattered enough to restructure the crate.
pub fn run() -> glib::ExitCode {
    let timeline = Timeline::start();

    // Before anything else can have anything to say. Startup is exactly when
    // a trace is worth having: an account that will not open, a store that
    // will not migrate and a keyring that will not answer all happen before
    // there is any UI to report them in.
    let config_path = postio_config::paths::config_path().ok();
    let logging = logging::init(
        &config_path
            .as_deref()
            .map(logging::config_at)
            .unwrap_or_default(),
    );
    // Held for the life of the process: dropping it stops the watch, and the
    // whole point of the `[logging]` section is raising the level on a
    // running Postio.
    let _log_watch = config_path.as_deref().and_then(|path| logging.watch(path));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "postio starting");

    if adw::init().is_err() {
        tracing::error!("no display; the UI needs a Wayland or X11 session");
        return glib::ExitCode::FAILURE;
    }
    timeline.mark(Phase::Init);

    // Fonts first, before any widget: see the module docs.
    if let Err(error) = fonts::install() {
        // Recoverable: the design degrades to system fallbacks, which is ugly
        // but usable, and refusing to start over a font would be worse.
        tracing::warn!(%error, "the embedded fonts did not install");
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
    // Filled in when the window is fed and an engine actually starts, which is
    // later than this and may not happen at all. `Refresh` reads it at the
    // moment it is pressed.
    let engine = refresh::EngineSlot::default();
    let builder = match &store {
        Some((database, _)) => actions::wire(
            Dispatcher::builder(),
            actions::Actions::new(database.clone(), state.clone()),
        ),
        // No store, so no local verb can do anything. An empty bus still
        // answers — every command comes back as "not wired up in this build" —
        // which is a sentence on screen rather than a key that does nothing.
        None => Dispatcher::builder(),
    };
    let bus = refresh::wire(builder, engine.clone(), state.clone()).build();

    // What the bus answers, asked before it is handed over: the window's
    // action seam carries *every* gesture, and the ones another consumer owns
    // must not come back as "not wired up in this build".
    let wired: Vec<postio_core::CommandId> = bus.wired().collect();

    // The runtime: the tokio threads every read is polled on and every command
    // is handled on.
    let (runtime, replies) = match Bridge::new(bus) {
        Ok((bridge, events)) => (Some(bridge), Some(events)),
        Err(error) => {
            tracing::error!(%error, "no runtime, so no mail");
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
            engine,
            ..Wiring::new(
                database,
                blobs,
                bridge.handle(),
                sink.clone(),
                bridge.commands(),
            )
        });
    application.connect_activate(move |application| {
        let Some(window) = application.active_window().and_downcast::<Window>() else {
            return;
        };
        if let Some(wiring) = &wiring {
            start_syncing(&window, wiring);
            let Some(feeds) = feed_the_window(&window, wiring) else {
                return;
            };
            // Every gesture the window produces from here on reaches a real
            // handler. Before this line the keymap, the palette and the
            // selection model all resolved correctly and then handed off to
            // nothing.
            commands::install(
                &window,
                &feeds,
                state.clone(),
                wiring.commands.clone(),
                wired.clone(),
            );
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
pub struct Wiring {
    /// The local store every pane reads through.
    pub database: Database,
    /// Bodies and attachments, content-addressed beside the database.
    pub blobs: BlobStore,
    /// The store as the frontend sees it: rows in, no SQL.
    pub store: Arc<dyn MailStore>,
    /// The runtime every read is polled on.
    pub runtime: tokio::runtime::Handle,
    /// Where the engine and the handlers report to.
    pub events: EventSink,
    /// The command bus.
    pub commands: postio_core::bridge::CommandSender,
    /// Where the engine goes once it is running, so `Refresh` can find it.
    pub engine: refresh::EngineSlot,
}

impl Wiring {
    /// Everything the panes need, over an already-open store.
    ///
    /// `runtime`, `events` and `commands` come from the `Bridge` in [`run`];
    /// a test supplies its own, which is the whole point of this being
    /// constructible from outside.
    pub fn new(
        database: Database,
        blobs: BlobStore,
        runtime: tokio::runtime::Handle,
        events: EventSink,
        commands: postio_core::bridge::CommandSender,
    ) -> Self {
        Wiring {
            store: Arc::new(SqliteStore::new(&database)),
            database,
            blobs,
            runtime,
            events,
            commands,
            engine: refresh::EngineSlot::default(),
        }
    }
}

/// Open the local store, or explain why there is none.
///
/// A missing or unreadable database is not a reason to refuse to start: the
/// window opens, says it has never synced, and stays usable for everything
/// that does not need mail. A mail client that will not open is worse than one
/// with nothing in it.
pub fn open_store() -> Option<(Database, BlobStore)> {
    let path = paths::store_path();
    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "cannot open the store");
            return None;
        }
    };
    // Beside the database, not inside it: bodies and attachments are
    // content-addressed files, and SQLite holds the key and the metadata.
    let blobs = match BlobStore::open(path.with_file_name("blobs")) {
        Ok(blobs) => blobs,
        Err(error) => {
            tracing::error!(%error, "cannot open the blob store");
            return None;
        }
    };
    if let Err(error) = ensure_search_index(&database) {
        // Recoverable: everything except search still works, and refusing to
        // open a mail client because its index would not build would be a
        // worse answer than opening one you cannot search.
        tracing::error!(%error, "the search index is unavailable");
    }

    Some((database, blobs))
}

/// Create the full-text index if it is not there, on every start.
///
/// The same contract `postio_storage::migrate` has, and for the same reason:
/// the schema is part of opening the store, not part of searching it. Once it
/// exists the metadata columns — subject, sender, recipients — are maintained
/// **by trigger**, exactly like the mailbox counts in migration 0003, so this
/// one call indexes every message already in the store and every one that
/// arrives after.
///
/// It was missing entirely. `postio_index::index::ensure_schema` documents
/// that it must run at startup and nothing ran it, so `search_documents` and
/// `messages_fts` did not exist on any real store and search had nothing to
/// search — `postio-x4e`, and the ninth instance of `postio-bl2`.
///
/// Message *bodies* are a separate matter: they live in the blob store, no
/// trigger can reach them, and `index_body` has to be called when a backfill
/// lands one. That half is still missing.
pub fn ensure_search_index(database: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let connection = database.connection()?;
    postio_index::index::ensure_schema(&connection)?;
    tracing::debug!("the search index is ready");
    Ok(())
}

/// Point the window's panes at the store.
///
/// Silent when there is no account yet: the sidebar already says what is true
/// — offline, never synced, no folders — and inventing an account to fill it
/// would be worse than an empty one. `postio-hiy` is the screen that creates
/// the first one.
pub fn feed_the_window(window: &Window, wiring: &Wiring) -> Option<postio_gtk::feed::Feeds> {
    let Some(account) = first_account(&wiring.database) else {
        tracing::info!(
            "no account configured; opening empty (see the provision example, or postio-hiy)"
        );
        return None;
    };
    // The account's *domain*, never the local part: enough to tell an iCloud
    // problem from a Fastmail one in a log somebody pastes into an issue,
    // and not enough to identify them.
    tracing::info!(
        account = account.id.get(),
        domain = account.address.domain().unwrap_or("unknown"),
        "opening account"
    );

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

    // Leaked for the same reason the engine is: the search surfaces live as
    // long as the window, and dropping the `View` here would unhook the
    // handlers that answer the box a moment after they were connected.
    if let Some(view) = search::install(window, wiring) {
        Box::leak(Box::new(view));
    }

    Some(feeds)
}

/// Bring the account's connection up and keep it up.
///
/// Split from [`feed_the_window`] because this half *dials a server* and that
/// half only reads the local store. A test that wants to know whether the
/// panes are wired has no business opening a socket, and before this split it
/// had no choice — which is one reason there was no such test.
///
/// Called first, so that the first thing the engine does — bring the link up,
/// drain whatever the last session left queued — is already under way while
/// the list is drawing.
pub fn start_syncing(window: &Window, wiring: &Wiring) {
    let Some(account) = first_account(&wiring.database) else {
        return;
    };
    let Some(sync) = engine::start(
        &account,
        &wiring.database,
        wiring.blobs.clone(),
        wiring.events.clone(),
    ) else {
        return;
    };
    // Leaked for the same reason the feeds are: it lives as long as the
    // process, and dropping it at exit would stop the engine a moment before
    // the process ends anyway.
    let sync: &'static _ = Box::leak(Box::new(sync));
    // `Refresh` is the one command that needs it, and it is pressed long
    // after the bus was built.
    wiring.engine.fill(sync.clone());
    seed_the_backfill(sync, wiring);
    fetch_what_is_opened(window, sync, wiring.runtime.clone());
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
                tracing::warn!(message = message.get(), %error, "cannot fetch that body");
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
    let mailboxes = match postio_storage::repository::MailboxRepository::new(&connection)
        .list_for_account(account.id)
    {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::error!(%error, "cannot read the account's folders");
            return;
        }
    };
    drop(connection);

    let selectable = mailboxes.iter().filter(|m| m.selectable).count();
    // Read *before* the engine has connected, so zero here is ordinary on a
    // first run — `postio_sync::discover` fills the table on link-up. Worth
    // saying anyway: a backfill seeded over no folders is the difference
    // between "still starting" and "nothing works".
    tracing::info!(known = mailboxes.len(), selectable, "folders known locally");

    for mailbox in mailboxes.into_iter().filter(|mailbox| mailbox.selectable) {
        wiring.runtime.spawn(async move {
            if let Err(error) = sync.seed_backfill(mailbox.id, BACKFILL_PER_MAILBOX).await {
                tracing::warn!(mailbox = %mailbox.path, %error, "cannot seed the backfill");
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
pub fn first_account(database: &Database) -> Option<postio_model::Account> {
    let connection = database
        .connection()
        .map_err(|error| tracing::error!(%error, "cannot read the accounts"))
        .ok()?;
    AccountRepository::new(&connection)
        .list_enabled()
        .map_err(|error| tracing::error!(%error, "cannot read the accounts"))
        .ok()?
        .into_iter()
        .next()
}
