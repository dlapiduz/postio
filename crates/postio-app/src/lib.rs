//! Postio — a local-first, keyboard-first email client.
//!
//! This is the composition root: the one crate that knows both halves of the
//! application exist. It opens the local store, starts the runtime, builds the
//! GTK frontend, and joins them.
//!
//! # Why it is its own crate
//!
//! `postio-gtk` must not depend on `rusqlite` or `io-imap` — the view layer
//! does no SQL and speaks no protocol, and `scripts/checks/check-crate-boundaries.py`
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

pub mod add_account;
pub mod commands;
pub mod compose;
pub mod export;
pub mod feed;
pub mod notifications;
pub mod onboarding;
pub mod reading;
pub mod search;
pub mod settings_accounts;
pub mod settings_credential;
mod settings_egress;
pub mod sidebar_backfill;

// The toolkit-free half of the composition root lives in `postio-session`, so
// that a frontend which is not GTK can link it (ADR 0010). Re-exported here
// rather than left for every call site to find, because `run` below reads as
// one startup sequence and it should not matter to the reader which side of
// the split each step came from.
pub use postio_session::{
    Wiring, actions, engine, ensure_search_index, first_account, logging, open_store, paths,
    refresh,
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

    // First run: config.toml does not exist yet. Postio's own defaults still
    // apply with nothing on disk (postio_config::Config::load_from_path says
    // so), so this changes discoverability, not behaviour -- Ctrl+E and a
    // file manager find a real file to read and edit rather than a blank
    // buffer. Before the watcher below, so there is nothing to race: the
    // watcher only needs to notice changes from here on, not this one.
    if let Some(path) = config_path.as_deref() {
        match postio_config::Config::seed_if_missing(path) {
            Ok(true) => tracing::info!(path = %path.display(), "seeded a starter config.toml"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "could not seed a starter config.toml"),
        }
    }

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

    // What the user is looking at, as the handlers see it. `commands::mirror`
    // brings it into step with the window in the instant before a command is
    // sent; nothing else writes it.
    let state = SharedState::default();

    // An installation has exactly one keyring, and every credential read goes
    // to the same instance: the store key here, and every account password
    // through `Wiring::secrets`.
    let secrets: std::sync::Arc<dyn postio_imap::secret::SecretStore> =
        std::sync::Arc::new(postio_imap::secret::KeyringSecretStore::default());

    // `[mailboxes]`, read once here alongside `[sync]` and for the same
    // reason `notifications::config_at` gives. Which folder this server calls
    // its archive is settled at discovery, and discovery runs inside the
    // engine, so this is the moment it has to be known.
    let mailbox_roles = config_path
        .as_deref()
        .map(postio_session::mailbox_roles_at)
        .unwrap_or_default();

    let context = Rc::new(Installation {
        secrets,
        state,
        mailbox_roles,
        sync_config: sync_config.clone(),
    });

    // The store's key, before the store. ADR 0014 Q3: a locked keyring means
    // the mail does not open, and there is no "open it unencrypted anyway" —
    // the same rule `secret.rs` has kept for passwords since it was written.
    //
    // Blocking, and only here: there is no window yet to freeze and no
    // runtime yet to defer to. The retry on the screen below runs the same
    // read on a thread, because by then there *is* a window.
    let first = postio_session::store_key_blocking(context.secrets.as_ref())
        .map_err(|error| error.to_string())
        .and_then(|key| open_with(&key, &context));
    let opened: Rc<std::cell::RefCell<Option<Opened>>> = Rc::new(std::cell::RefCell::new(None));
    // Whether `open_or_onboard` has already run for this window (#514): a
    // second `activate` -- a second launch of a single-instance app just
    // raises the window -- must not open a second set of engines and feeds
    // over the first. See `open_or_onboard`'s own doc comment for why it is
    // the one that checks and sets this, not `present` here.
    let fed: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));
    let refused = match first {
        Ok(ready) => {
            *opened.borrow_mut() = Some(ready);
            None
        }
        Err(reason) => {
            // Safe verbatim: no `SecretError` carries key material. The same
            // sentence goes to the log and to the screen, because the log is
            // for a bug report and the screen is for the person who has to
            // unlock their keyring.
            tracing::error!(reason, "the store did not open");
            Some(reason)
        }
    };

    let application = app::build_with(timeline);

    // Connected *after* the frontend's own handler, so the window it makes is
    // already there to be fed. Signal handlers run in the order they were
    // connected, which is the whole of the arrangement.
    application.connect_activate({
        let opened = Rc::clone(&opened);
        let context = Rc::clone(&context);
        let fed = Rc::clone(&fed);
        move |application| {
            let Some(window) = application.active_window().and_downcast::<Window>() else {
                return;
            };
            // Exists before the first notification can, and re-registering on
            // a second `activate` (a second launch raising the window) just
            // replaces it with itself.
            notifications::install_action(application, &window);
            present(&window, &opened, &context, refused.clone(), &fed);
        }
    });

    let code = application.run();
    // Taken rather than borrowed: `shutdown` consumes the bridge, and by here
    // the window is gone and nothing else is going to read this.
    if let Some(ready) = opened.borrow_mut().take() {
        // The clean-shutdown marker (#491): a next start that finds it will
        // leave a parked draft parked instead of recovering it as a crash.
        postio_session::end_session(&ready.wiring.database);
        ready.bridge.shutdown();
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
///
/// # `fed` makes a second `activate` a no-op
///
/// A single-instance `gtk::Application` delivers a second `activate` to the
/// primary process when a second launch just means "raise the window" —
/// and `run()`'s handler called this every time, unconditionally. Nothing
/// downstream of it was idempotent: a second `start_syncing` would run a
/// second set of engines against the store `open_account` already opened,
/// and a second `search::install` puts two handlers on the same
/// `connect_run` — see [`Wired`]'s own doc comment for what that one cost.
///
/// `fed.replace(true)` both reads and sets in the one call a single-threaded
/// main loop needs to close the race a plain check-then-set would leave:
/// this must not merely record that wiring *finished*, because the keyring
/// lookup below is asynchronous, and a second `activate` arriving while the
/// first is still waiting on it must not start a second lookup and a second
/// eventual [`open_account`]/[`onboarding::install`] of its own. Marking it
/// the instant this is entered — win or lose the race that already cannot
/// happen on one thread, either way there is exactly one way in.
#[allow(clippy::too_many_arguments)]
pub fn open_or_onboard(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<postio_core::CommandId>,
    events: Rc<std::cell::RefCell<Option<EventStream>>>,
    notifier: notifications::Notifier,
    fed: Rc<std::cell::Cell<bool>>,
) {
    if fed.replace(true) {
        return;
    }
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
                // The real transport is built here, in the composition root,
                // rather than inside the probe: that is what lets a test
                // drive the same `install` over a mock (#282).
                Startup::Onboard(repairing) => onboarding::install(
                    &window,
                    &wiring,
                    state,
                    wired,
                    events,
                    notifier,
                    repairing.map(|account| *account),
                    std::sync::Arc::new(
                        postio_imap::discovery::PimalayaTransport::new()
                            .with_egress(wiring.egress.clone()),
                    ),
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

    // Which message is on screen: one cell, read by the pane that paints it
    // and by the composer that replies to it. Two separately-updated copies
    // is exactly what #325 was.
    let showing = reading::Showing::default();

    compose::install(
        window,
        account.id,
        wiring.database.clone(),
        wiring.blobs.clone(),
        wiring.runtime.clone(),
        showing.clone(),
    );

    // The reading pane. After `compose::install`, because the two share the
    // pane and the window wires their swap when the composer is installed.
    reading::install(window, wiring, &feeds, showing);

    // Dragging messages out to another application. Nothing is written until
    // a drop actually asks, so this costs nothing until it is used.
    export::install(window, wiring);

    // The settings panel's account rows: enable/disable, remove-with-undo.
    settings_accounts::install(window, wiring);
    // And its connection list: the egress log, auditable (#151).
    settings_egress::install(window, wiring);

    // A folder's own context menu: skip/resume background backfill (ADR
    // 0016, #350).
    sidebar_backfill::install(window, wiring);

    // *Add account*, from the palette or its binding. Here rather than in
    // `open_account` because it is a surface over the shell, and the shell
    // is what this function builds -- an application with no account to feed
    // is already on the first-run screen, where adding a second one is not a
    // question anybody can ask.
    add_account::install(window, wiring);

    // Leaked for the same reason the engine is: the search surfaces live as
    // long as the window, and dropping the `View` here would unhook the
    // handlers that answer the box a moment after they were connected.
    let search = search::install(window, wiring, &feeds).map(|view| &*Box::leak(Box::new(view)));

    catch_up_the_body_index(wiring);
    reclaim_disk(wiring);

    Some(Wired { feeds, search })
}

/// Give back the disk nothing is using any more, out of the way.
///
/// # Why this had to be added rather than fixed
///
/// `BlobStore::collect_garbage` and `BlobStore::purge_temporary` were both
/// written, tested and documented, and neither had a production caller (#416).
/// The blob store's own module docs describe garbage collection as *the*
/// mechanism that keeps blobs from leaking — "a sweep cannot drift out of sync
/// with the data" — and `MessageRepository::delete` removes a message's row
/// without touching its blobs precisely because that sweep is supposed to
/// follow.
///
/// With nothing calling it, **deleting mail freed nothing, ever**, and a
/// `UIDVALIDITY` reset — which wipes and re-syncs an entire mailbox — orphaned
/// every blob in it at once, permanently.
///
/// # Two sweeps, two different costs
///
/// The debris purge is one `read_dir` of a directory that is empty in the
/// ordinary case, so it runs first and inline. Garbage collection walks the
/// whole blob tree, which on a backfilled archive is a great many files, so it
/// goes on a worker for the same reason [`catch_up_the_body_index`] does: a
/// mail client that will not draw until it has counted its own files has
/// traded the wrong thing.
///
/// `BLOB_GRACE_PERIOD`, never `Duration::ZERO`. A blob is written before the
/// row that references it is committed, so inside that window a healthy blob
/// looks exactly like an orphan, and a sweep without the grace period deletes
/// the body of a message that is mid-fetch.
///
/// Once per start, not on a timer. Orphans are produced by deletes, moves and
/// resyncs — none of which happen fast enough to be worth a schedule, and all
/// of which will still be there next time.
fn reclaim_disk(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    wiring.runtime.spawn_blocking(move || {
        if let Err(error) = postio_session::purge_fetch_debris(&blobs) {
            tracing::warn!(%error, "could not remove debris from unfinished fetches");
        }
        // Dragged-out mail, which is a privacy question rather than only a
        // disk one: these are full plaintext copies of messages, outside the
        // blob store, in a directory nothing audits (#278). The grace period
        // is the guard -- see `reclaim_drag_exports` -- not a tuning knob.
        match postio_session::reclaim_drag_exports(
            &postio_session::paths::export_dir(),
            postio_session::DRAG_EXPORT_GRACE_PERIOD,
        ) {
            Ok(0) => {}
            Ok(removed) => tracing::debug!(removed, "reclaimed dragged-out exports"),
            Err(error) => tracing::warn!(%error, "could not reclaim dragged-out exports"),
        }
        if let Err(error) = postio_session::reclaim_orphaned_blobs(
            &database,
            &blobs,
            postio_session::BLOB_GRACE_PERIOD,
        ) {
            // Recoverable, and the same judgement the body index makes: a mail
            // client that could not tidy up still reads mail, and the next
            // start tries again.
            tracing::warn!(%error, "could not reclaim blobs nothing references");
        }
    });
}

/// Index the bodies that were already on this machine, out of the way.
///
/// `postio_sync::backfill::fetch_body` indexes each body as it lands, so
/// everything fetched from now on is covered. This is the mail that arrived
/// before that call existed: `index_body` was written, tested and benched and
/// nothing ever called it, so `search_documents.body` was empty on every
/// message in every real store and search matched metadata only (#327).
///
/// # On the runtime, and after the window
///
/// The first pass over an existing archive reads a blob per message, which is
/// minutes of I/O on a large one — nothing a startup budget of 500 ms can
/// hold. So it is spawned, exactly as `seed_the_backfill` spawns its seeding,
/// and search fills in behind a window that is already usable. Every pass
/// after the first costs one query that finds nothing.
///
/// Here rather than in `start_syncing` because it dials nothing: a store
/// opened with no account, or with the network down, still has bodies on
/// disk and should still become searchable.
///
/// `spawn_blocking`, not `spawn`: this is synchronous SQLite and synchronous
/// blob reads from beginning to end, and a blocking call inside a tokio task
/// stalls whatever else that worker was meant to poll.
fn catch_up_the_body_index(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    wiring.runtime.spawn_blocking(move || {
        if let Err(error) = postio_session::index_local_bodies(&database, &blobs) {
            // Recoverable, and the same judgement `ensure_search_index`
            // makes: a mail client whose body search is behind still reads
            // mail, and the next start tries again.
            tracing::warn!(%error, "could not index the bodies already on disk");
        }
    });
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

    let engines = match engine::start_all(
        &accounts,
        &wiring.database,
        wiring.blobs.clone(),
        wiring.events.clone(),
        wiring.secrets.clone(),
        wiring.mailbox_roles.clone(),
        wiring.backfill,
        &wiring.egress,
    ) {
        Ok(engines) => engines,
        Err(refusal) => {
            // A sentence, not a hang. Starting some of the engines would
            // leave the rest of the accounts looking permanently offline
            // with nothing explaining why (#183).
            tracing::error!(%refusal, "not starting the sync engines");
            return;
        }
    };

    for (account, sync) in engines {
        adopt_engine(window, wiring, account, sync);
    }
}

/// Bring `account` into an application that is already running (#64).
///
/// The other caller of [`adopt_engine`], and the whole of ADR 0012 Q2: an
/// account created by the add-account dialogue must reach the same state as
/// one that was in the store before the window existed, without a restart.
/// What differs between the two is only what a caller can see here —
/// [`start_syncing`] asks the connection budget about a set of accounts
/// being started from nothing, and this asks it about a set being joined —
/// so the refusal comes back rather than going to the log: the surface that
/// asked for this is on screen and can say so.
///
/// Not a feed and not a sidebar entry yet. Both are still keyed to one
/// account (`feed_the_window`'s `first_account`, `Sidebar::set_account`),
/// and giving a second account somewhere to appear is #1's own work — this
/// is the entry point that stops that being the only thing missing.
pub fn attach_account(
    window: &Window,
    wiring: &Wiring,
    account: &postio_model::Account,
) -> Result<(), engine::StartupRefusal> {
    // Counted after the write, so the joining account is in it: the pool has
    // to serve every enabled account, not every account that had an engine
    // when the window opened.
    let accounts = enabled_accounts(&wiring.database).len();
    let started = engine::start_joining(
        account,
        accounts,
        &wiring.database,
        wiring.blobs.clone(),
        wiring.events.clone(),
        wiring.secrets.clone(),
        wiring.mailbox_roles.clone(),
        wiring.backfill,
        &wiring.egress,
    )?;
    if let Some(sync) = started {
        adopt_engine(window, wiring, account.id, sync);
    }
    // The surfaces that list accounts, now that there is one more. Nothing
    // else reads the account table while the window is up; when something
    // does, this is where it joins.
    settings_accounts::refresh(window, &wiring.database);
    Ok(())
}

/// Hand one started engine to the window: the backfill it seeds, the body
/// fetch an opened row asks for, and the slot `Refresh` reads.
///
/// One function for both the startup pass and an account that joined later,
/// so "the application started with this account" and "the application
/// gained it" cannot drift apart in what an engine is wired to (ADR 0012
/// Q2).
fn adopt_engine(
    window: &Window,
    wiring: &Wiring,
    account: postio_model::AccountId,
    sync: postio_runtime::Engine,
) {
    // Leaked for the same reason the feeds are: it lives as long as the
    // process, and dropping it at exit would stop the engine a moment
    // before the process ends anyway.
    let sync: &'static _ = Box::leak(Box::new(sync));
    // `Refresh` is the one command that needs it, and it is pressed long
    // after the bus was built. The first engine fills the slot; the
    // others are reached through their own account's work.
    wiring.engine.fill(sync.clone());
    seed_the_backfill(account, sync, wiring);
    fetch_what_is_opened(window, sync, wiring.runtime.clone());
}

/// Every account that participates in sync.
///
/// ADR 0005 Q3: the first account is not special. This replaces
/// `first_account` on the sync path — any code that treats one account
/// differently fails exactly once, in the field.
fn enabled_accounts(database: &Database) -> Vec<postio_model::Account> {
    let Ok(connection) = database.connection() else {
        tracing::error!("cannot read the accounts");
        return Vec::new();
    };
    postio_storage::repository::AccountRepository::new(&connection)
        .list_enabled()
        .unwrap_or_else(|error| {
            tracing::error!(%error, "cannot read the accounts");
            Vec::new()
        })
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
/// to fetch it again on the wire when the user opens it. Seeding *again* is
/// the engine's own business now: it tops the queue up when it drains and
/// re-seeds a folder whose sync changed something, so this is the first batch
/// rather than the only one (#318).
fn seed_the_backfill(
    account: postio_model::AccountId,
    sync: &'static postio_runtime::Engine,
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

/// How many bodies this startup pass queues per mailbox.
///
/// The first batch, not the horizon. The engine tops the queue up whenever it
/// drains and seeds a folder again whenever a sync changes it, so a mailbox is
/// covered by however many batches it takes — this only decides how much of it
/// is on the wire before the engine's own loop takes over. It read as a
/// horizon for the life of the project, because nothing ever seeded a second
/// time and everything below the newest 200 messages of a folder waited to be
/// opened (#318).
///
/// `postio_sync::backfill::BackfillPolicy::seed_batch` is what the engine uses
/// for every batch after this one, and is where the size of them belongs.
const BACKFILL_PER_MAILBOX: u32 = 200;

/// The choices about *this installation* that outlive a failed start.
///
/// Named for what it holds rather than for when it runs: `Startup` in this
/// module is already the enum that decides between an account and onboarding.
///
/// Held so a retry can rebuild everything the first attempt could not: which
/// keyring, which folder is the archive, how hard to sync. None of it depends
/// on the store, which is exactly why it survives the store not opening.
struct Installation {
    secrets: std::sync::Arc<dyn postio_imap::secret::SecretStore>,
    state: SharedState,
    mailbox_roles: postio_model::RoleOverrides,
    sync_config: postio_config::SyncConfig,
}

/// Everything downstream of the store key.
///
/// Built in one go because it is one dependency chain — the store feeds the
/// command bus, the bus feeds the runtime, the runtime feeds the wiring — and
/// a half-built one is not a state anything downstream knows how to handle.
/// Either the mail opens or a screen says why.
struct Opened {
    wiring: Wiring,
    /// What the bus answers, asked before it was handed over: the window's
    /// action seam carries *every* gesture, and the ones another consumer
    /// owns must not come back as "not wired up in this build".
    wired: Vec<postio_core::CommandId>,
    /// Taken on the first `activate`. `EventStream` is not `Clone` — there is
    /// one queue and exactly one reader of it — and `activate` can fire again
    /// when a second launch raises the window.
    events: Rc<std::cell::RefCell<Option<EventStream>>>,
    /// The tokio threads every read is polled on. Held to the end of `run`,
    /// which is what shuts it down.
    bridge: postio_core::bridge::Bridge,
}

/// Opens the store under `key` and builds the bus, the runtime and the wiring.
///
/// The whole of what a locked keyring was standing between the user and, so
/// that a retry has one function to call rather than a sequence to reproduce.
fn open_with(
    key: &postio_storage::key::StoreKey,
    context: &Installation,
) -> Result<Opened, String> {
    let (database, blobs) = open_store(key)?;

    // Filled in when the window is fed and an engine actually starts, which
    // is later than this and may not happen at all. `Refresh` reads it at the
    // moment it is pressed.
    let engine = refresh::EngineSlot::default();
    let builder = actions::wire(
        Dispatcher::builder(),
        actions::Actions::new(database.clone(), context.state.clone()),
    );
    let bus = refresh::wire(builder, engine.clone(), context.state.clone()).build();
    let wired: Vec<postio_core::CommandId> = bus.wired().collect();

    // Every producer's events, and every consumer's view of them. The bus's
    // handlers and the sync engine are two producers; the window is one
    // subscriber, and ADR 0013 exists so that an MCP server can be a second
    // one without stealing the window's repaints.
    let hub = EventHub::new();
    // The engine is not a command handler, so the bridge never hands it a
    // sink; it holds one of its own on the same hub.
    let sink = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(bus, hub.sink())
        .map_err(|error| {
            tracing::error!(%error, "no runtime, so no mail");
            format!("Postio could not start its runtime: {error}")
        })?;

    let wiring = Wiring {
        engine,
        ..Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands())
            .with_mailbox_roles(context.mailbox_roles.clone())
            .with_backfill(postio_session::backfill_policy(&context.sync_config))
            .with_secrets(context.secrets.clone())
    };

    Ok(Opened {
        wiring,
        wired,
        // One subscription rather than the `Vec<Option<EventStream>>` this
        // used to be: fan-in is the hub's now, so the window no longer
        // collects a stream per producer by hand.
        events: Rc::new(std::cell::RefCell::new(Some(hub.subscribe("window")))),
        bridge,
    })
}

/// Puts either the mail or the reason there is none in front of the user.
///
/// Called on `activate`, and again by the retry on the screen below — which
/// is why it is a function rather than the body of a closure.
fn present(
    window: &Window,
    opened: &Rc<std::cell::RefCell<Option<Opened>>>,
    context: &Rc<Installation>,
    refused: Option<String>,
    fed: &Rc<std::cell::Cell<bool>>,
) {
    // Borrowed, checked, and dropped before anything else runs: the retry
    // closure installed below writes this same cell, and a borrow left open
    // across it is a `borrow_mut` panic waiting for whoever edits this next.
    let ready = opened.borrow().is_some();
    if ready {
        let held = opened.borrow();
        let ready = held.as_ref().expect("just checked");
        let notifier = notifications::Notifier::new(
            ready.wiring.database.clone(),
            ready.wiring.store.clone(),
            ready.wiring.runtime.clone(),
            context.sync_config.clone(),
        );
        open_or_onboard(
            window,
            &ready.wiring,
            context.state.clone(),
            ready.wired.clone(),
            Rc::clone(&ready.events),
            notifier,
            Rc::clone(fed),
        );
        return;
    }

    // No store. ADR 0014 Q3 means that is a hard stop rather than a degraded
    // mode, so the window says so and offers the one action that can change
    // it. #404: this was a toast, which vanished while the condition did not.
    let screen = postio_gtk::unavailable::Unavailable::new();
    screen.set_reason(
        refused
            .as_deref()
            .unwrap_or("Postio could not open its local store."),
    );
    window.set_content(Some(&screen));
    screen.focus_retry();

    screen.connect_retry({
        let screen = screen.clone();
        let window = window.clone();
        let opened = Rc::clone(opened);
        let context = Rc::clone(context);
        let fed = Rc::clone(fed);
        move || {
            screen.set_busy(true);
            // On a thread, not on this one. Reading the keyring is a D-Bus
            // round trip against a service that may be showing the user a
            // passphrase prompt of its own, and `store_key_blocking` waits
            // out `KEYRING_TIMEOUT` for it. There is no runtime to defer to
            // here — building one is what failed — so this is the plain
            // thread the situation calls for.
            let (sender, receiver) = async_channel::bounded(1);
            let secrets = context.secrets.clone();
            std::thread::spawn(move || {
                let read = postio_session::store_key_blocking(secrets.as_ref())
                    .map_err(|error| error.to_string());
                let _ = sender.send_blocking(read);
            });

            glib::spawn_future_local({
                let screen = screen.clone();
                let window = window.clone();
                let opened = Rc::clone(&opened);
                let context = Rc::clone(&context);
                let fed = Rc::clone(&fed);
                async move {
                    let read = match receiver.recv().await {
                        Ok(read) => read,
                        Err(_) => Err("Postio stopped reading the keyring before                                        it answered."
                            .to_owned()),
                    };
                    screen.set_busy(false);
                    // The key is only half of it: the store still has to
                    // open, and `open_with` is the same function the first
                    // attempt ran, so a retry that succeeds continues exactly
                    // as a normal start would.
                    match read.and_then(|key| open_with(&key, &context)) {
                        Ok(ready) => {
                            tracing::info!("the store opened on a retry");
                            *opened.borrow_mut() = Some(ready);
                            present(&window, &opened, &context, None, &fed);
                        }
                        Err(reason) => {
                            tracing::warn!(reason, "the store still did not open");
                            screen.set_reason(&reason);
                            screen.focus_retry();
                        }
                    }
                }
            });
        }
    });
}

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

/// Deletes every account "Remove" (in the settings panel, #464) has marked
/// but not yet actually removed, cascading to its mail.
///
/// Called once, at the top of [`startup_route`], before anything decides
/// which account to open or starts an engine for one — the boundary ADR
/// 0005 Q6a chose specifically so a crash before the undo toast expires
/// leaves the row exactly as marked, not half deleted. Failing to read or
/// write is logged and otherwise ignored: a reap that cannot run this
/// launch gets another chance next launch, and the account stays out of
/// `list_enabled` either way.
fn reap_pending_accounts(database: &Database) {
    let Ok(connection) = database.connection() else {
        return;
    };
    if let Err(error) =
        postio_storage::repository::AccountRepository::new(&connection).reap_pending_deletions()
    {
        tracing::error!(%error, "could not reap an account marked for removal");
    }
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
    reap_pending_accounts(database);
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
    async fn a_pending_deletion_account_is_reaped_before_startup_decides_anything() {
        // #464: "Remove" in the settings panel only marks the row, so
        // something has to actually delete it -- once, at the next launch,
        // before an engine could otherwise start against it.
        let (database, _key) = provisioned();
        let connection = database.connection().expect("a connection");
        let id = postio_storage::repository::AccountRepository::new(&connection)
            .list()
            .expect("list")[0]
            .id;
        postio_storage::repository::AccountRepository::new(&connection)
            .mark_pending_deletion(id)
            .expect("mark");
        drop(connection);

        assert!(
            matches!(
                startup_route(&database, &MemorySecretStore::new()).await,
                Startup::Onboard(None)
            ),
            "a pending-deletion account is not there to open or to prefill from"
        );

        let connection = database.connection().expect("a connection");
        assert!(
            postio_storage::repository::AccountRepository::new(&connection)
                .get(id)
                .expect("get")
                .is_none(),
            "startup_route must actually reap it, not merely skip past it"
        );
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
