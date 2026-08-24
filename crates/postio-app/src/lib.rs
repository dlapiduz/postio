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
pub mod export;
pub mod feed;
pub mod logging;
pub mod notifications;
pub mod onboarding;
pub mod paths;
pub mod reading;
pub mod refresh;
pub mod search;

use std::rc::Rc;
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

    // `[sync]`'s notification settings, read once here rather than kept
    // live — see `notifications::config_at`.
    let sync_config = config_path
        .as_deref()
        .map(notifications::config_at)
        .unwrap_or_default();

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
    // when a second launch raises the window. `Rc` rather than a plain
    // `RefCell`: `onboarding::install` holds its own clone and drains these
    // from its own closure, once its screen has created the account
    // `activate` did not find one for at startup.
    let streams = Rc::new(std::cell::RefCell::new(vec![Some(events), replies]));

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
        // Exists before the first notification can, and re-registering on a
        // second `activate` (a second launch raising the window) just
        // replaces it with itself.
        notifications::install_action(application, &window);
        let Some(wiring) = &wiring else {
            return;
        };
        let notifier = notifications::Notifier::new(
            wiring.database.clone(),
            wiring.store.clone(),
            wiring.runtime.clone(),
            sync_config.clone(),
        );

        open_or_onboard(
            &window,
            wiring,
            state.clone(),
            wired.clone(),
            Rc::clone(&streams),
            notifier,
        );
    });

    let code = application.run();
    if let Some(bridge) = runtime {
        bridge.shutdown();
    }
    code
}

/// Open the account, or ask for the one thing that is missing.
///
/// The whole of `activate`'s decision, in a function rather than in a
/// closure, so that something other than a running application can drive
/// it. `postio-bl2` is the bead for what happens when the composition root
/// is only reachable by launching the binary: every layer under it was
/// tested and eight capabilities were wired to nothing.
///
/// Which branch this takes depends on the keyring, and the keyring is a
/// tokio future — so it is asked on the runtime and answered on the main
/// context, the crossing `feed.rs` describes. The window is already up by
/// then, which is the point: a blocking keyring read would trade
/// `postio-67`'s wrong guess for a startup that stalls on a locked keyring.
#[allow(clippy::too_many_arguments)]
pub fn open_or_onboard(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<postio_core::CommandId>,
    streams: Rc<std::cell::RefCell<Vec<Option<postio_core::bridge::EventStream>>>>,
    notifier: notifications::Notifier,
) {
    let (sender, receiver) = async_channel::bounded(1);
    {
        let database = wiring.database.clone();
        let secrets = wiring.secrets.clone();
        wiring.runtime.spawn(async move {
            let _ = sender
                .send(startup_route(&database, secrets.as_ref()).await)
                .await;
        });
    }
    glib::spawn_future_local({
        let window = window.clone();
        let wiring = wiring.clone();
        async move {
            let route = receiver.recv().await.unwrap_or_else(|_| {
                // The runtime went away before it answered. There is no
                // account this process can open without one, and the screen
                // at least says what Postio is waiting for.
                tracing::error!("the runtime stopped before startup could read the keyring");
                Startup::Onboard(None)
            });
            match route {
                Startup::Ready(_) => {
                    open_account(&window, &wiring, &state, &wired, &streams, &notifier)
                }
                // `postio-hiy`: nothing to feed yet, or nothing that can
                // authenticate. The screen replaces the window's content and
                // finishes the same sequence `open_account` runs, once it has
                // written the two things an account needs.
                Startup::Onboard(repairing) => onboarding::install(
                    &window,
                    &wiring,
                    state,
                    wired,
                    streams,
                    notifier,
                    repairing.map(|account| *account),
                ),
            }
        }
    });
}

/// Point the window at `wiring` and wire every gesture to a real handler.
///
/// The tail end of `run()`'s `activate` handler, factored out so
/// [`onboarding::install`]'s successful submission can reach the exact same
/// sequence once it has created the account `run()` did not find at
/// startup — the account this depends on did not exist yet, but everything
/// else about bringing a window up is identical.
fn open_account(
    window: &Window,
    wiring: &Wiring,
    state: &SharedState,
    wired: &[postio_core::CommandId],
    streams: &Rc<std::cell::RefCell<Vec<Option<postio_core::bridge::EventStream>>>>,
    notifier: &notifications::Notifier,
) {
    start_syncing(window, wiring);
    let Some(Wired { feeds, .. }) = feed_the_window(window, wiring) else {
        return;
    };
    // Every gesture the window produces from here on reaches a real handler.
    // Before this line the keymap, the palette and the selection model all
    // resolved correctly and then handed off to nothing.
    commands::install(
        window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        wired.to_vec(),
    );
    // Everything either half has to say reaches the panes here: a mailbox the
    // server disagreed with, a body that arrived, an archive that landed. Two
    // queues because there are two producers — the engine and the bus — and
    // one reader each.
    for stream in streams.borrow_mut().iter_mut().filter_map(Option::take) {
        commands::drain(window, &feeds, stream, notifier.clone());
    }
}

/// What the frontend needs, once there is a store to give it.
#[derive(Clone)]
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
    /// Where account passwords live.
    ///
    /// A part rather than something the modules that need it construct, for
    /// the reason `engine.rs` gives about every other part: which keyring
    /// this installation uses is a choice about *this installation*, and a
    /// module that reaches for `KeyringSecretStore::default()` itself cannot
    /// be driven by a test without a Secret Service session. Both credential
    /// paths — the one onboarding writes and the one startup reads — hang
    /// off this.
    pub secrets: Arc<dyn postio_imap::secret::SecretStore>,
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
            secrets: Arc::new(postio_imap::secret::KeyringSecretStore::default()),
        }
    }

    /// The same wiring, reading and writing passwords somewhere else.
    ///
    /// The seam a test needs: `MemorySecretStore` stands in for a keyring
    /// that has no D-Bus session behind it, and `MemorySecretStore::locked`
    /// for one nobody has unlocked.
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_imap::secret::SecretStore>) -> Self {
        self.secrets = secrets;
        self
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

/// Everything `feed_the_window` wires up, for whoever has to drive it.
///
/// The `View` is handed back rather than only leaked because it is the far
/// side of the search: the preview, the scope column and the refine chips all
/// hang off it, and a caller that wants to *check* any of them would otherwise
/// have to call `search::install` a second time. Two installs put two handlers
/// on the box's `connect_run`, the query answers into the view the caller
/// cannot see, and every search surface reads empty — which cost this bead an
/// afternoon of chasing a wiring bug that was not there.
pub struct Wired {
    /// The message list, the folders and the status line.
    pub feeds: postio_gtk::feed::Feeds,
    /// The search surfaces, or `None` when search could not be installed.
    ///
    /// `'static` because it is leaked: these live as long as the window, and
    /// dropping the `View` unhooks the handlers a moment after they are
    /// connected.
    pub search: Option<&'static postio_gtk::search::View>,
}

/// Point the window's panes at the store.
///
/// Silent when there is no account yet: the sidebar already says what is true
/// — offline, never synced, no folders — and inventing an account to fill it
/// would be worse than an empty one. `postio-hiy` is the screen that creates
/// the first one.
pub fn feed_the_window(window: &Window, wiring: &Wiring) -> Option<Wired> {
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
        sources.clone(),
    );
    // The same store, read as a set of hits rather than a window over a
    // mailbox. Set here rather than inside `install_feeds` because whether a
    // window has a search is the composition root's business: postio-gtk
    // deliberately holds no opinion, and a `Feed` without this goes on
    // showing mailboxes and ignores `Event::SearchResults`.
    feeds.messages.set_result_source(sources);

    compose::install(
        window,
        account.id,
        wiring.database.clone(),
        wiring.blobs.clone(),
        wiring.runtime.clone(),
    );

    // The reading pane. After `compose::install`, because the two share the
    // pane and the window wires their swap when the composer is installed.
    reading::install(window, wiring);

    // Leaked for the same reason the engine is: the search surfaces live as
    // long as the window, and dropping the `View` here would unhook the
    // handlers that answer the box a moment after they were connected.
    let search = search::install(window, wiring, &feeds).map(|view| &*Box::leak(Box::new(view)));

    Some(Wired { feeds, search })
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
        wiring.secrets.clone(),
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

/// What startup should do with the account this installation has, if any.
///
/// The distinction the 0.1.0 routing did not make. It asked whether an
/// account *row* existed, and one row was enough — so an installation whose
/// credential write had failed opened an account that could not
/// authenticate, could not sync, and could not be repaired from inside the
/// app, because onboarding is the only thing that writes a credential and
/// onboarding never ran again.
///
/// An account is something to open only when the store holds a row **and**
/// the keyring will give up a password for it. That also covers the
/// credential being deleted, or the keyring being reset, later — which no
/// amount of care at write time prevents.
#[derive(Debug)]
pub enum Startup {
    /// Open it: there is a row, and a password to authenticate with.
    Ready(Box<postio_model::Account>),
    /// Show the first-run screen, prefilled from the row when there is one.
    ///
    /// `Some` is a *repair*: the account is configured and only its
    /// credential is missing, so the screen already knows the address and
    /// the servers and needs a password. `None` is a genuine first run.
    Onboard(Option<Box<postio_model::Account>>),
}

/// Decide which of the two startup does.
///
/// Async because reading the keyring is: `KeyringSecretStore` reaches the
/// Secret Service over D-Bus and bounds the round trip with a timeout, so
/// this must be polled on the engine runtime and answered over a channel —
/// never awaited on the GTK main context. `feed.rs` explains the rule.
///
/// A keyring that will not answer therefore costs the window a moment, not
/// the session: the timeout inside `retrieve` turns silence into an error,
/// and an error means onboarding rather than a window that never decides.
pub async fn startup_route(
    database: &Database,
    secrets: &dyn postio_imap::secret::SecretStore,
) -> Startup {
    let Some(account) = first_account(database) else {
        return Startup::Onboard(None);
    };
    let key = postio_imap::secret::AccountKey::new(account.address.address.clone());
    // The account's domain, never the local part, for the same reason
    // `feed_the_window` logs only that.
    let domain = account.address.domain().unwrap_or("unknown").to_owned();
    match secrets.retrieve(&key).await {
        Ok(password) if !password.is_empty() => Startup::Ready(Box::new(account)),
        Ok(_) => {
            tracing::warn!(%domain, "the keyring holds an empty password; asking for it again");
            Startup::Onboard(Some(Box::new(account)))
        }
        Err(error) => {
            // Safe to log verbatim: no `SecretError` carries a password.
            tracing::warn!(%domain, %error, "no usable password for the account; asking for it again");
            Startup::Onboard(Some(Box::new(account)))
        }
    }
}

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

#[cfg(test)]
mod tests {
    //! What startup decides, without a display or a keyring.
    //!
    //! [`startup_route`] is the whole of the decision `run()`'s `activate`
    //! handler makes, factored out so it can be driven against a
    //! [`MemorySecretStore`] rather than only against a real Secret Service
    //! on a real first run.

    use super::*;
    use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

    /// A store with one enabled account in it, and the key its credential
    /// would be filed under.
    fn provisioned() -> (Database, AccountKey) {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("a connection");
        let account = postio_storage::test_support::account(&connection);
        drop(connection);
        let key = AccountKey::new(account.address.address.clone());
        (database, key)
    }

    #[tokio::test]
    async fn an_account_with_its_password_is_opened() {
        let (database, key) = provisioned();
        let secrets = MemorySecretStore::new();
        secrets
            .store(&key, &Password::new("app-specific"))
            .await
            .expect("the credential should store");

        assert!(matches!(
            startup_route(&database, &secrets).await,
            Startup::Ready(_)
        ));
    }

    #[tokio::test]
    async fn an_account_whose_password_never_landed_goes_back_to_onboarding() {
        // The bug this test exists for: onboarding wrote the row, the keyring
        // write failed, and every launch after that opened an account that
        // could not authenticate and could not be repaired.
        let (database, _) = provisioned();

        match startup_route(&database, &MemorySecretStore::new()).await {
            Startup::Onboard(Some(prefill)) => assert_eq!(
                prefill.address.address, "test@example.com",
                "the screen has to come back prefilled, not empty"
            ),
            other => panic!("a row with no credential is not an account to open: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_locked_keyring_goes_back_to_onboarding_too() {
        // Not the same fault, and the same dead end: a credential that cannot
        // be read is a credential the account does not have. The store here
        // *has* the item; it just will not open.
        let (database, key) = provisioned();
        let locked = MemorySecretStore::locked();
        assert!(
            locked.retrieve(&key).await.is_err(),
            "the double has to refuse, or this test cannot fail"
        );

        assert!(matches!(
            startup_route(&database, &locked).await,
            Startup::Onboard(Some(_))
        ));
    }

    #[tokio::test]
    async fn an_empty_password_is_no_password() {
        let (database, key) = provisioned();
        let secrets = MemorySecretStore::new();
        secrets
            .store(&key, &Password::new(""))
            .await
            .expect("the credential should store");

        assert!(matches!(
            startup_route(&database, &secrets).await,
            Startup::Onboard(Some(_))
        ));
    }

    #[tokio::test]
    async fn a_fresh_installation_has_nothing_to_prefill_with() {
        let database = postio_storage::test_support::memory();

        assert!(matches!(
            startup_route(&database, &MemorySecretStore::new()).await,
            Startup::Onboard(None)
        ));
    }
}
