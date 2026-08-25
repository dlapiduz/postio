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

pub mod commands;
pub mod compose;
pub mod export;
pub mod feed;
pub mod notifications;
pub mod onboarding;
pub mod reading;
pub mod search;

// The toolkit-free half of the composition root lives in `postio-session`, so
// that a frontend which is not GTK can link it (ADR 0010). Re-exported here
// rather than left for every call site to find, because `run` below reads as
// one startup sequence and it should not matter to the reader which side of
// the split each step came from.
pub use postio_session::{
    Wiring, actions, enabled_accounts, engine, engine_budget, ensure_search_index, logging,
    open_store, paths, refresh,
};

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_core::bridge::{Bridge, EventHub, EventStream};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::startup::{Phase, Timeline};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::Database;

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

    // Every producer's events, and every consumer's view of them. The bus's
    // handlers and the sync engine are two producers; the window is one
    // subscriber, and ADR 0013 exists so that an MCP server can be a second
    // one without stealing the window's repaints.
    let hub = EventHub::new();
    // The engine is not a command handler, so the bridge never hands it a
    // sink; it holds one of its own on the same hub.
    let sink = hub.sink();

    // The runtime: the tokio threads every read is polled on and every command
    // is handled on.
    let runtime = match Bridge::builder().build_with_events(bus, hub.sink()) {
        Ok(bridge) => Some(bridge),
        Err(error) => {
            tracing::error!(%error, "no runtime, so no mail");
            None
        }
    };

    // Taken on the first `activate`. `EventStream` is not `Clone` — there is
    // one queue and exactly one reader of it — and `activate` can fire again
    // when a second launch raises the window. `Rc` rather than a plain
    // `RefCell`: `onboarding::install` holds its own clone and drains it from
    // its own closure, once its screen has created the account `activate` did
    // not find one for at startup.
    //
    // One subscription rather than the `Vec<Option<EventStream>>` this used to
    // be: fan-in is the hub's now, so the window no longer collects a stream
    // per producer by hand.
    let events = Rc::new(std::cell::RefCell::new(Some(hub.subscribe("window"))));

    let application = app::build_with(timeline);

    // Connected *after* the frontend's own handler, so the window it makes is
    // already there to be fed. Signal handlers run in the order they were
    // connected, which is the whole of the arrangement.
    // `[mailboxes]`, read once here alongside `[sync]` and for the same
    // reason `notifications::config_at` gives. Which folder this server calls
    // its archive is settled at discovery, and discovery runs inside the
    // engine, so this is the moment it has to be known.
    let mailbox_roles = config_path
        .as_deref()
        .map(postio_session::mailbox_roles_at)
        .unwrap_or_default();
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
            .with_mailbox_roles(mailbox_roles.clone())
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
            Rc::clone(&events),
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
    events: Rc<std::cell::RefCell<Option<EventStream>>>,
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
                    open_account(&window, &wiring, &state, &wired, &events, &notifier)
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
                    events,
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
    events: &Rc<std::cell::RefCell<Option<EventStream>>>,
    notifier: &notifications::Notifier,
) {
    start_syncing(window, wiring);
    let Some(Wired { account, feeds, .. }) = feed_the_window(window, wiring) else {
        return;
    };
    // One account on screen, said once to both halves (#182): `AppState` is
    // what the bus consults, and the window is what filters the palette and
    // the cheat sheet. `quiet`: the panes are being fed in this same breath,
    // so there is nothing to repaint from the announcement.
    {
        let (quiet, _) = postio_core::bridge::event_channel();
        state.update(&quiet, |app| {
            app.open_scope(postio_core::state::Scope::Account(account))
        });
    }
    window.set_scope(postio_core::state::Scope::Account(account));
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
    // server disagreed with, a body that arrived, an archive that landed. One
    // queue, because the hub fans both producers in before the window sees
    // them — and `take`, because a second `activate` must not drain a stream
    // that is already being drained.
    if let Some(stream) = events.borrow_mut().take() {
        commands::drain(window, &feeds, stream, notifier.clone());
    }
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
    /// The account the panes were pointed at.
    ///
    /// What the caller needs to set the scope (#182): the window's palette
    /// and cheat sheet filter on it, and `AppState` mirrors it, so both
    /// halves agree about which account is on screen without re-reading the
    /// store to ask.
    pub account: postio_model::ids::AccountId,
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
    // `.first()` here is the *panes'* single-account cut, not the sync
    // engine's: every enabled account syncs (#183), and this is the one call
    // site that still has to pick one to draw, because the sidebar does not
    // wear several yet. #185 is where this choice goes to die; until then it
    // is made here, visibly, rather than behind a helper that makes "the
    // first one" look like an answer.
    let Some(account) = enabled_accounts(&wiring.database).into_iter().next() else {
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
    reading::install(window, wiring, &feeds);

    // Dragging messages out to another application. Nothing is written until
    // a drop actually asks, so this costs nothing until it is used.
    export::install(window, wiring);

    // Leaked for the same reason the engine is: the search surfaces live as
    // long as the window, and dropping the `View` here would unhook the
    // handlers that answer the box a moment after they were connected.
    let search = search::install(window, wiring, &feeds).map(|view| &*Box::leak(Box::new(view)));

    Some(Wired {
        account: account.id,
        feeds,
        search,
    })
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
    let accounts = enabled_accounts(&wiring.database);
    if accounts.is_empty() {
        return;
    }

    // Each engine holds a connection from the pool for the length of a sync
    // pass. Starting more engines than the pool can serve is not a slow
    // start, it is a deadlock: the extra engine waits forever for a
    // connection another pass is holding, and nothing ever says why. So the
    // refusal is a sentence, up front (#183, ADR 0005 Q3). `open_store`
    // sizes the pool from the account count, so this fires only when the
    // store changed under a running process -- accounts added since open.
    let budget = engine_budget(wiring.database.pool().max_connections());
    if accounts.len() > budget {
        let sentence = format!(
            "{} accounts are enabled but the store was opened with room to \
             sync {budget}. Not starting any engine, because starting some \
             and not others would sync whichever came first and silently \
             skip the rest. Restart Postio to re-size the pool.",
            accounts.len()
        );
        tracing::error!(accounts = accounts.len(), budget, "engine budget exceeded");
        wiring
            .events
            .emit(postio_core::Event::Error { message: sentence });
        return;
    }

    // The account the panes are drawing (`feed_the_window`'s `.first()`), so
    // the row-activation hook below asks the engine whose mail is actually
    // on screen. The other engines sync all the same -- that is the point.
    let mut on_screen: Option<&'static postio_runtime::Engine> = None;

    for account in &accounts {
        let Some(sync) = engine::start(
            account,
            &wiring.database,
            wiring.blobs.clone(),
            wiring.events.clone(),
            wiring.secrets.clone(),
            wiring.mailbox_roles.clone(),
        ) else {
            // `engine::start` has already said why (no transport, say).
            // The other accounts still get their engines: one account that
            // cannot start must not take the rest offline with it.
            continue;
        };
        // Leaked for the same reason the feeds are: it lives as long as the
        // process, and dropping it at exit would stop the engine a moment
        // before the process ends anyway.
        let sync: &'static _ = Box::leak(Box::new(sync));
        // `Refresh` resolves whose engine to ask from the scope (#182), long
        // after the bus was built.
        wiring.engine.fill(account.id, sync.clone());
        seed_the_backfill(sync, account.id, wiring);
        if on_screen.is_none() {
            on_screen = Some(sync);
        }
    }

    if let Some(sync) = on_screen {
        fetch_what_is_opened(window, sync, wiring.runtime.clone());
    }
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
fn seed_the_backfill(
    sync: &'static postio_runtime::Engine,
    account: postio_model::ids::AccountId,
    wiring: &Wiring,
) {
    let Ok(connection) = wiring.database.connection() else {
        return;
    };
    let mailboxes = match postio_storage::repository::MailboxRepository::new(&connection)
        .list_for_account(account)
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
    let accounts = enabled_accounts(database);
    if accounts.is_empty() {
        return Startup::Onboard(None);
    }
    // Any account that can authenticate opens the window (#183): every
    // enabled account syncs, so one broken credential must not hold the rest
    // hostage behind the repair screen. Only when *no* account is usable
    // does this route to repair -- for the first broken one, which is the
    // one the user most likely just typed.
    let mut broken: Option<postio_model::Account> = None;
    for account in accounts {
        let key = postio_imap::secret::AccountKey::new(account.address.address.clone());
        // The account's domain, never the local part, for the same reason
        // `feed_the_window` logs only that.
        let domain = account.address.domain().unwrap_or("unknown").to_owned();
        match secrets.retrieve(&key).await {
            Ok(password) if !password.is_empty() => {
                return Startup::Ready(Box::new(account));
            }
            Ok(_) => {
                tracing::warn!(%domain, "the keyring holds an empty password; will ask again");
                broken.get_or_insert(account);
            }
            Err(error) => {
                // Safe to log verbatim: no `SecretError` carries a password.
                tracing::warn!(%domain, %error, "no usable password; will ask again");
                broken.get_or_insert(account);
            }
        }
    }
    Startup::Onboard(broken.map(Box::new))
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
