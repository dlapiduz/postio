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
pub mod frontend;
pub mod notifications;
pub mod onboarding;
pub mod orientation;
pub mod reading;
mod recipients;
pub mod remote;
pub mod search;
pub mod settings_accounts;
pub mod settings_credential;
mod settings_egress;
mod settings_privacy;
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
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::startup::{Phase, Timeline};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::Store;

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

    let state = SharedState::default();

    // An installation has exactly one keyring, and every credential read goes
    // to the same instance: the store key here, and every account password
    // through `Wiring::secrets`.
    let secrets: std::sync::Arc<dyn postio_account::secret::SecretStore> =
        std::sync::Arc::new(postio_account::secret::KeyringSecretStore::default());

    // `[mailboxes]`, read once here alongside `[sync]` and for the same
    // reason `notifications::config_at` gives. Which folder this server calls
    // its archive is settled at discovery, and discovery runs inside the
    // engine, so this is the moment it has to be known.
    let mailbox_roles = config_path
        .as_deref()
        .map(postio_session::mailbox_roles_at)
        .unwrap_or_default();

    // `[storage] max_bytes`, read once here for the same reason `[mailboxes]`
    // is: `reclaim_disk` is spawned from `feed_the_window`, which runs before
    // anything is watching the file.
    let storage_ceiling = config_path
        .as_deref()
        .and_then(postio_session::storage_ceiling_at);

    let context = Rc::new(Installation {
        secrets,
        state,
        mailbox_roles,
        sync_config: sync_config.clone(),
        storage_ceiling,
        early: std::cell::RefCell::new(None),
    });
    // The store starts opening now, before GTK does anything (#1604): the
    // keyring and the engine's open of an encrypted file need nothing from
    // the window, and used to wait until it was built and presented. The
    // first `open_the_store`, from `activate`, takes this open over.
    context.start_opening();
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

    // **The window first, and the store behind it.** Until #1114 this read
    // the keyring and opened the store here, before `app::build_with` was
    // called at all -- so there was no application, let alone a window,
    // until the store had already succeeded or been refused. That is fine
    // when opening takes 30ms and indefensible when it does not: a schema
    // migration held a launch on the live install for 12.6s, and a keyring
    // prompt held another for 28s, both with nothing whatever on screen.
    //
    // Nothing about the failure path changes. ADR 0014 Q3 still means a
    // store that will not open is a hard stop rather than a degraded mode;
    // what moves is *when* that is decided, and therefore that the refusal
    // now replaces the content of a window somebody is already looking at
    // (#404's screen, exactly as before).
    let opened: Rc<std::cell::RefCell<Option<Opened>>> = Rc::new(std::cell::RefCell::new(None));
    // Whether `open_or_onboard` has already run for this window (#514): a
    // second `activate` -- a second launch of a single-instance app just
    // raises the window -- must not open a second set of engines and feeds
    // over the first. See `open_or_onboard`'s own doc comment for why it is
    // the one that checks and sets this, not `present` here.
    let fed: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));
    // And whether the store is already being opened. `fed` cannot answer
    // this: it is set at the far end of a chain that only starts once the
    // store has landed, so a second `activate` arriving during the open
    // would find it still false and start a second thread, a second runtime
    // and a second set of engines over the same file.
    let opening: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));

    let application = app::build_with(timeline.clone());

    // Connected *after* the frontend's own handler, so the window it makes is
    // already there to be fed. Signal handlers run in the order they were
    // connected, which is the whole of the arrangement.
    application.connect_activate({
        let opened = Rc::clone(&opened);
        let context = Rc::clone(&context);
        let fed = Rc::clone(&fed);
        let opening = Rc::clone(&opening);
        let timeline = timeline.clone();
        move |application| {
            postio_session::blocking::now(async {
                let Some(window) = application.active_window().and_downcast::<Window>() else {
                    return;
                };
                // Exists before the first notification can, and re-registering on
                // a second `activate` (a second launch raising the window) just
                // replaces it with itself.
                notifications::install_action(application, &window);
                if opened.borrow().is_some() {
                    // A second launch raising a window that already has its mail.
                    present(&window, &opened, &context, None, &fed).await;
                    return;
                }
                if opening.replace(true) {
                    return;
                }
                open_the_store(&window, &opened, &context, &fed, &timeline);
            })
        }
    });

    let code = application.run();

    // The sync engines first, and before anything else here: they are the one
    // thing in this process still writing to the database on a thread of
    // their own. Letting `main` return with one of them mid-commit leaves a
    // write torn by the process exit for the store engine to recover -- and
    // that engine is pre-1.0, so waiting for the pass to finish rather than
    // trusting its young WAL recovery is why `Engine` keeps its `JoinHandle`.
    // (Under SQLCipher this was sharper still: a coredump through libcrypto's
    // atexit teardown, which the pure-Rust engine cannot reproduce.)
    //
    // Bounded: `stop_retained` waits a few seconds per engine and gives up
    // rather than holding a closed window open on a stalled network read.
    postio_runtime::stop_retained();

    // Taken rather than borrowed: `shutdown` consumes the bridge, and by here
    // the window is gone and nothing else is going to read this.
    if let Some(ready) = opened.borrow_mut().take() {
        // The clean-shutdown marker (#491): a next start that finds it will
        // leave a parked draft parked instead of recovering it as a crash.
        //
        // Blocked on, because the GTK main loop has already returned and
        // there is nothing left to keep responsive -- this is the last write
        // of the process.
        ready.host.stop();
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
pub async fn open_or_onboard(
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
                    // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
                    // network work it reaches is spawned onto the runtime and answers over a
                    // channel -- `onboarding::probe_with_offer` is the shape -- and what is
                    // left is store reads, whose futures this engine makes self-contained.
                    // Measured rather than assumed: `app_suite::glib_main_context` opens a
                    // store and reads it on this context with no runtime anywhere, and fails
                    // loudly if that stops being true.
                    open_account(&window, &wiring, &state, &wired, &events, &notifier).await
                }
                // `postio-hiy`: nothing to feed yet, or nothing that can
                // authenticate. The screen replaces the window's content and
                // finishes the same sequence `open_account` runs, once it has
                // written the two things an account needs.
                // The real transport is built here, in the composition root,
                // rather than inside the probe: that is what lets a test
                // drive the same `install` over a mock (#282).
                Startup::Onboard(repairing) => {
                    onboarding::install(
                        &window,
                        &wiring,
                        state,
                        wired,
                        events,
                        notifier,
                        repairing.map(|account| *account),
                        std::sync::Arc::new(
                            postio_account::discovery::PimalayaTransport::new()
                                .with_egress(wiring.egress.clone()),
                        ),
                        std::sync::Arc::new(postio_account::oauth::browser::SystemBrowserOpener),
                    )
                    // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
                    // network work it reaches is spawned onto the runtime and answers over a
                    // channel -- `onboarding::probe_with_offer` is the shape -- and what is
                    // left is store reads, whose futures this engine makes self-contained.
                    // Measured rather than assumed: `app_suite::glib_main_context` opens a
                    // store and reads it on this context with no runtime anywhere, and fails
                    // loudly if that stops being true.
                    .await
                }
            }
            // Both branches, because both are a usable UI: mail to read, or
            // the screen that asks for the account there is none of. The
            // budget closes on the next frame either way, which since #1114
            // is a later frame than the one the window first appeared in —
            // the window arrives early and this is when it is worth
            // something.
            window.report_usable();
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
async fn open_account(
    window: &Window,
    wiring: &Wiring,
    state: &SharedState,
    wired: &[postio_core::CommandId],
    events: &Rc<std::cell::RefCell<Option<EventStream>>>,
    notifier: &notifications::Notifier,
) {
    open_account_for(
        window,
        &frontend::Frontend::in_process(wiring),
        state,
        wired,
        events,
        notifier,
    )
    .await;
}

/// [`open_account`], for a window whose store's owner may be another
/// process: the owner there syncs, and is asked to start.
pub(crate) async fn open_account_for(
    window: &Window,
    frontend: &frontend::Frontend,
    state: &SharedState,
    wired: &[postio_core::CommandId],
    events: &Rc<std::cell::RefCell<Option<EventStream>>>,
    notifier: &notifications::Notifier,
) {
    // **Storage first, and the network only once there is something to look
    // at.** This read `start_syncing` then `feed_the_window`, so opening the
    // account connected to the server, authenticated and listed its folders
    // before a single stored message reached the list. Measured on a real
    // account: the first frame was 2282ms of a 2474ms startup, against a
    // 500ms budget, and the log says plainly what it was waiting for:
    //
    //     56.558  opening account
    //     57.085  connected and authenticated      <- a round trip
    //     57.258  listed the server's folders      <- another
    //     58.713  first frame
    //
    // `docs/PRODUCT.md` §18 budgets startup at 500ms and the architecture
    // note says the UI never awaits the network. Startup was the one place
    // that did, and it awaited it before drawing anything.
    //
    // A profile with no account painted in 104ms, which looked like the
    // store's size and was not: it had no server to call.
    //
    // The mail is already on disk. Everything below this line reads it, and
    // none of it needs a connection.
    let Some(Wired { feeds, .. }) = feed_the_window_for(window, frontend).await else {
        return;
    };

    // And the network, after the frame the stored mail is drawn in.
    //
    // `on_first_frame` and not an idle callback: idle means "when the loop is
    // free", which is a promise about the loop; this needs to mean "once the
    // person can see their mail", which is a promise about the screen.
    match &frontend.wiring {
        Some(wiring) => postio_gtk::startup::on_first_frame(window, {
            let window = window.clone();
            let wiring = wiring.clone();
            move || postio_session::blocking::now(start_syncing(&window, &wiring))
        }),
        // The owner's engines. It started the ones it had when it opened
        // the store; this asks for any it has not -- an account the
        // first-run screen just wrote.
        None => {
            postio_gtk::startup::on_first_frame(window, {
                let client = frontend.client.clone();
                move || client.start_sync()
            });
            // The one body the person is waiting for goes to the front of
            // the backfill, as `fetch_what_is_opened` sends it in-process.
            window.list().connect_activated({
                let client = frontend.client.clone();
                move |row| client.fetch_body(row.id)
            });
            // New mail: the owner decides, and tells this window only if it
            // is the one elected to deliver (`postio_host::notify`).
            if let Some(application) = window.application() {
                notifications::deliver_from(&application, window, &feeds, &frontend.client);
            }
        }
    }
    // Every gesture the window produces from here on reaches a real handler.
    // Before this line the keymap, the palette and the selection model all
    // resolved correctly and then handed off to nothing.
    commands::install(
        window,
        &feeds,
        state.clone(),
        frontend.commands.clone(),
        wired.to_vec(),
    );
    // Everything either half has to say reaches the panes here: a mailbox the
    // server disagreed with, a body that arrived, an archive that landed. One
    // queue, because the hub fans both producers in before the window sees
    // them — and `take`, because a second `activate` must not drain a stream
    // that is already being drained.
    if let Some(stream) = events.borrow_mut().take() {
        commands::drain(window, &feeds, stream, notifier.clone(), state.clone());
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
pub async fn feed_the_window(window: &Window, wiring: &Wiring) -> Option<Wired> {
    // The window's surfaces read through a client of the store's owner
    // (ADR 0041). Over this wiring: the host is in this process, and the
    // client alone keeps it alive.
    feed_the_window_for(window, &frontend::Frontend::in_process(wiring)).await
}

/// [`feed_the_window`], for a window whose store's owner may be another
/// process: every surface reads through `frontend`'s client.
pub async fn feed_the_window_for(window: &Window, frontend: &frontend::Frontend) -> Option<Wired> {
    // Everything from here to the return is synchronous main-thread work,
    // and the first frame is waiting on all of it. The two marks around it
    // are what let a startup trace say so: before #1479 the whole stretch
    // between the window and the paint was one `first frame` phase, and on a
    // real store it was 84% of a 1250 ms startup with nothing to say about
    // which part. A window nothing is measuring ignores both marks.
    window.mark_startup(postio_gtk::startup::Phase::Account);

    // The composer's editing surface is ADR 0003's `WebView`, and its first
    // load starts a WebKit web process: measured, 28.7ms on the first open of
    // a composer against 0.2ms on every one after it. Without this it all
    // lands on the first message somebody sits down to write, which is the one
    // composition they are most likely to notice (#1216).
    //
    // On an idle turn, and not here, because startup has 500ms to reach a
    // usable window and this is thirty of them spent on something nobody has
    // asked for yet. Before the account check below, because a window with no
    // account is a window in onboarding, and the composition after that one is
    // still the first one.
    //
    // The composition root's call to make, like the search source below:
    // `postio-gtk` must not decide on its own that every window it builds is
    // worth a web process.
    glib::idle_add_local_once({
        let window = glib::object::ObjectExt::downgrade(window);
        move || {
            if let Some(window) = window.upgrade() {
                window.composer().warm();
                // The reading pane's web process too, now that its reader no
                // longer starts one while it is built (#1603): on this idle
                // turn rather than before the first frame, and before the
                // first message is opened into it.
                window.reader().warm();
            }
        }
    });

    // Whatever else this call does, it is handing the window a store. The
    // window learns it here rather than from every caller, so a surface that
    // asks the registry what is available gets the same answer as the pane
    // that is about to fill with mail (#1114). `present` says the same thing
    // one step earlier, for the onboarding branch that never reaches here.
    window.set_store_open(true);

    let client = frontend.client.clone();

    // Every account this window needs to know about, in one read: the one it
    // opens on, the strip's, the scope switch's addresses and the one new
    // mail is written from. Enabled ones only, in creation order -- the
    // order the host lists them in, and `AppState::accounts`' too.
    let enabled: Vec<postio_model::Account> = match client.accounts().await {
        Ok(accounts) => accounts
            .into_iter()
            .filter(|account| account.enabled)
            .collect(),
        Err(error) => {
            tracing::error!(%error, "cannot read the accounts: {error}");
            Vec::new()
        }
    };
    let Some(account) = enabled.first().cloned() else {
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

    let sources = feed::Sources::new(
        std::sync::Arc::new(client.clone()),
        frontend.runtime.clone(),
    );
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

    // The accounts strip, and what a click on it does (#185).
    //
    // Absent below two accounts — `set_accounts` decides that, not this — so
    // for everybody with one account this is a length check and nothing on
    // screen changes. The order is the host's own listing, which is
    // ascending id, the same order `AppState::accounts` uses: the hue is the
    // position, so it has to be the same list in both places or an account
    // changes colour depending on which surface is drawing it.
    let named: Vec<(postio_model::AccountId, String)> = enabled
        .iter()
        .map(|account| (account.id, account.display_name.clone()))
        .collect();
    //
    // `offer_unified: true` since `ListScope::Unified` gave the row somewhere
    // to lead (#185). It opens on an account rather than on Unified: the
    // scope a person left in is not remembered yet, and one account's inbox
    // is the smaller surprise on a cold start.
    window
        .sidebar()
        .set_accounts(&named, postio_core::state::Scope::Account(account.id), true);
    // And the window, which is a separate thing from the strip: the strip is
    // what a person clicks, `Window::scope` is what decides whether a command
    // needing somewhere in *one* account to put a message is offered at all
    // (`Requirement::SingleAccount`). Nothing set it before this, and
    // `AccountScope`'s own default is Unified -- so every window, including
    // every single-account one, has been hiding "Move to…" from the palette
    // and the cheat sheet since the requirement was added.
    window.set_scope(postio_core::state::Scope::Account(account.id));
    // With more than one account the sidebar draws a section each, so the feed
    // has to read every tree rather than the current one (#185). `install_feeds`
    // has already opened the current account's; this re-points it, and only
    // when there is a second account to be worth the extra read.
    if named.len() > 1 {
        let ids: Vec<postio_model::AccountId> = named.iter().map(|(id, _)| *id).collect();
        feeds
            .folders
            .open_sections(&ids, account.id, &account.address.address);
    }
    // The status line's manual sync (#495). Through `Window::act`, so the
    // pointer and `F5`/`R` reach one verb rather than two implementations of
    // it -- and so the command's own filters, the palette and the cheat
    // sheet all go on describing the same thing.
    window.sidebar().connect_refresh_requested(glib::clone!(
        #[weak]
        window,
        move || window.act(postio_core::Command::Refresh)
    ));
    window.sidebar().connect_scope_selected({
        let feeds = feeds.clone();
        let sidebar = glib::object::ObjectExt::downgrade(&window.sidebar());
        let window_for_scope = glib::object::ObjectExt::downgrade(window);
        let ids: Vec<postio_model::AccountId> = named.iter().map(|(id, _)| *id).collect();
        let addresses: Vec<(postio_model::AccountId, String)> = enabled
            .iter()
            .map(|account| (account.id, account.address.address.clone()))
            .collect();
        move |scope| {
            // What is available follows the scope wherever it goes, so this
            // is first and unconditional: both branches below change what the
            // list is showing, and a window still claiming the old scope
            // offers the wrong verbs for the new one.
            if let Some(window) = window_for_scope.upgrade() {
                window.set_scope(scope);
            }
            // Re-point the folder feed, which re-reads that account's tree
            // and, through its own loaded handler, opens its inbox. Nothing
            // here reaches into the list: the folders are what the list
            // follows, so there is one path rather than two that can
            // disagree about which account is on screen.
            let Some(id) = scope.account() else {
                // Unified is the exception, and it has to be: it is a view
                // over every account rather than a folder in one, so there
                // is no tree to re-point and no inbox for a loaded handler
                // to open. The list is addressed directly, and the folder
                // highlight is cleared because no folder is showing -- a
                // sidebar still pointing at Inbox while the list draws every
                // account's mail is the app disagreeing with itself about
                // where the user is.
                feeds.messages.open(postio_model::ListScope::Unified);
                if let Some(sidebar) = sidebar.upgrade() {
                    sidebar.clear_folder_selection();
                }
                return;
            };
            let Some((_, address)) = addresses.iter().find(|(candidate, _)| *candidate == id)
            else {
                return;
            };
            // `open_sections` and not `open` once there is more than one
            // account: `open` clears the sections, so switching scope would
            // redraw the sidebar as a single tree and every other account's
            // folders would vanish. What changes here is which account is
            // *current*, not which are *drawn* -- the two are separate
            // arguments for exactly this reason (#185).
            if ids.len() > 1 {
                feeds.folders.open_sections(&ids, id, address);
            } else {
                feeds.folders.open(id, address);
            }
        }
    });

    // Which message is on screen: one cell, read by the pane that paints it
    // and by the composer that replies to it. Two separately-updated copies
    // is exactly what #325 was.
    let showing = reading::Showing::default();

    // The account a new message comes from is the one marked default, which
    // is not necessarily the one the window opened on (#960, #1161): the
    // marker means "new messages come from here" and nothing about order.
    let composing =
        postio_session::composing_account(&enabled).map_or(account.id, |chosen| chosen.id);
    // The window's own client: the host tells every frontend a send moved a
    // row, and this window hears it, so the composer announces nothing
    // itself.
    compose::install_with(
        window,
        composing,
        client.clone(),
        frontend.runtime.clone(),
        showing.clone(),
        None,
    )
    .await;

    // The reading pane. After `compose::install`, because the two share the
    // pane and the window wires their swap when the composer is installed.
    reading::install_for(
        window,
        frontend.runtime.clone(),
        frontend.events.clone(),
        client.clone(),
        &feeds,
        showing,
    )
    .await;

    // ADR 0012 Q4: the first-run keyboard orientation, after the first sync.
    // Installed here rather than in `postio-gtk` because the two questions
    // it turns on -- has this been seen, and has a sync finished -- are a
    // store read and an engine event, and the view layer has neither.
    orientation::install(window, &frontend.runtime, client.clone(), &feeds).await;

    // Dragging messages out to another application. Nothing is written until
    // a drop actually asks, so this costs nothing until it is used.
    export::install_for(window, frontend.runtime.clone(), client.clone());

    // Which accounts are rebuilding their local search index right now
    // (#981) -- shared between the settings panel, which owns the set, and
    // search, which reads it to raise a search outcome's own corpus caveat
    // while a rebuild is running.
    let reindexing: settings_accounts::Reindexing = Default::default();

    // The settings panel's account rows: enable/disable, remove-with-undo,
    // rebuild-index, and each account's mailbox role map.
    settings_accounts::install_for(window, frontend, client.clone(), reindexing.clone(), &feeds)
        .await;
    // And its connection list: the egress log, auditable (#151).
    settings_egress::install(window, client.clone()).await;
    // The privacy pane's unsubscribe-activation log (#971).
    settings_privacy::install(window, client.clone()).await;

    // A folder's own context menu: skip/resume background backfill (ADR
    // 0016, #350).
    sidebar_backfill::install(window, client.clone()).await;

    // *Add account*, from the palette or its binding. Here rather than in
    // `open_account` because it is a surface over the shell, and the shell
    // is what this function builds -- an application with no account to feed
    // is already on the first-run screen, where adding a second one is not a
    // question anybody can ask.
    add_account::install_for(window, frontend).await;

    // Leaked for the same reason the engine is: the search surfaces live as
    // long as the window, and dropping the `View` here would unhook the
    // handlers that answer the box a moment after they were connected.
    let search = search::install_for(window, &frontend.events, client.clone(), &feeds, reindexing)
        .await
        .map(|view| &*Box::leak(Box::new(view)));

    // The body indexer: a catch-up pass now, then one batched write after
    // each burst of `BodyLoaded` on the wiring's hub. Every body reaches the
    // search index through it and nothing else -- neither the store nor the
    // fetch writes the row -- see `postio_session::spawn_body_indexer`. Here,
    // with the other idle passes, because this is the call `run` makes and
    // the one `search_index::opening_the_window_indexes_local_bodies_without_
    // being_asked` proves reaches a person: a store opened with no account,
    // or with the network down, still has bodies on disk and still becomes
    // searchable.
    //
    // **After the first frame, and a moment after it** (#1604). All four are
    // catch-up work nobody is waiting on, and they used to start at the same
    // instant as the first page read, on a runtime of two workers -- the
    // journal had the WAL truncation landing 20 ms after the first page was
    // asked for. The indexer's catch-up pass also covers any `BodyLoaded` it
    // was not yet subscribed to hear.
    //
    // Only where the store's owner is this process: `postio-daemon` runs
    // them itself, a moment after it opens the store.
    if let Some(wiring) = &frontend.wiring {
        let idle = wiring.clone();
        postio_gtk::startup::on_first_frame(window, move || {
            let wiring = idle.clone();
            glib::timeout_add_local_once(IDLE_PASSES_AFTER_FIRST_FRAME, move || {
                postio_host::maintenance::spawn_idle_passes(&wiring);
            });
        });
    }

    // Live `[storage] max_bytes` (#929): the ceiling is read once at startup
    // through `Wiring::storage_ceiling` -- this is the other half. Lowering
    // it evicts without a restart; raising it needs nothing beyond the next
    // pass reading the new number. Off the main thread, the same reason
    // `reclaim_disk`'s own ceiling pass above is: `evict_to_fit` reads and
    // deletes blobs.
    window.connect_storage_changed({
        let client = client.clone();
        move |max_bytes| client.storage_ceiling(max_bytes)
    });

    // The panes are pointed at the store and every gesture has a handler.
    // Nothing between here and the compositor's first frame is Postio's, so
    // whatever `first frame` costs from this mark on is GTK's own paint.
    window.mark_startup(postio_gtk::startup::Phase::Feeds);

    Some(Wired { feeds, search })
}

/// How long after the first frame the idle passes start: long enough for
/// the first pages and the reading pane to have had the runtime to
/// themselves, short enough that a store opened to search is searchable
/// within a breath.
const IDLE_PASSES_AFTER_FIRST_FRAME: std::time::Duration = std::time::Duration::from_millis(750);

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
pub async fn start_syncing(window: &Window, wiring: &Wiring) {
    let accounts = enabled_accounts(&wiring.database).await;
    if accounts.is_empty() {
        return;
    }

    let engines = match engine::start_all(&accounts, wiring).await {
        Ok(engines) => engines,
        Err(refusal) => {
            // A sentence, not a hang. Starting some of the engines would
            // leave the rest of the accounts looking permanently offline
            // with nothing explaining why (#183).
            tracing::error!(%refusal, "not starting the sync engines: {refusal}");
            return;
        }
    };

    for (_, sync) in engines {
        adopt_engine(window, wiring, sync).await;
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
pub async fn attach_account(
    window: &Window,
    wiring: &Wiring,
    account: &postio_model::Account,
) -> Result<(), engine::StartupRefusal> {
    // Counted after the write, so the joining account is in it: the pool has
    // to serve every enabled account, not every account that had an engine
    // when the window opened.
    let accounts = enabled_accounts(&wiring.database).await.len();
    let started = engine::start_joining(account, accounts, wiring).await?;
    if let Some(sync) = started {
        adopt_engine(window, wiring, sync).await;
    }
    // The surfaces that list accounts, now that there is one more. Nothing
    // else reads the account table while the window is up; when something
    // does, this is where it joins. Read through a client of the store's
    // owner, as the panel reads (ADR 0041); this signature is the one the
    // add-account dialogue and its test call, so it connects its own.
    let frontend = frontend::Frontend::in_process(wiring);
    settings_accounts::refresh(window, &frontend, &frontend.client).await;
    Ok(())
}

/// Hand one started engine to the window: the backfill it seeds, the body
/// fetch an opened row asks for, and the slot `Refresh` reads.
///
/// One function for both the startup pass and an account that joined later,
/// so "the application started with this account" and "the application
/// gained it" cannot drift apart in what an engine is wired to (ADR 0012
/// Q2).
async fn adopt_engine(window: &Window, wiring: &Wiring, sync: postio_runtime::Engine) {
    // Retained rather than leaked. It does live as long as the session, but
    // "dropping it at exit would stop the engine a moment before the process
    // ends anyway" -- which is what the leak was for -- stopped being safe
    // once that moment could leave a write torn mid-commit for a pre-1.0
    // engine to recover. `run` calls `stop_retained` before it returns. See
    // `postio_runtime::engine::EngineThread`.
    postio_runtime::retain(sync.clone());
    // `Refresh` is the one command that needs it, and it is pressed long
    // after the bus was built. The first engine fills the slot; the
    // others are reached through their own account's work.
    wiring.engine.fill(sync.clone());
    // No seeding of the body queue from here: the engine tops it up itself,
    // in priority order, the moment it has a connection (#1593). Fifteen
    // `SeedBackfill` jobs sent from spawned tasks used to land during the
    // first wave, and every one of them stopped the lanes.
    fetch_what_is_opened(window, sync, wiring.runtime.clone()).await;
}

/// Every account that participates in sync.
///
/// ADR 0005 Q3: the first account is not special. This replaces
/// `first_account` on the sync path — any code that treats one account
/// differently fails exactly once, in the field.
async fn enabled_accounts(database: &Store) -> Vec<postio_model::Account> {
    let Ok(connection) = database.read().await else {
        tracing::error!("cannot read the accounts");
        return Vec::new();
    };
    postio_storage::repository::AccountRepository::new(&connection)
        .list_enabled()
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "cannot read the accounts: {error}");
            Vec::new()
        })
}

/// Jump a message to the front of the backfill when it is opened.
///
/// The one body the user is actually waiting for. Everything else in the
/// queue is a guess about what they will want next; this is not a guess, so
/// it goes to the front of the queue rather than the back.
async fn fetch_what_is_opened(
    window: &Window,
    sync: postio_runtime::Engine,
    runtime: tokio::runtime::Handle,
) {
    window.list().connect_activated(move |row| {
        let message = row.id;
        // A handle per activation: `Engine` is a channel sender and an `Arc`,
        // so cloning is cheap, and the closure is `Fn` rather than `FnOnce`.
        let sync = sync.clone();
        runtime.spawn(async move {
            if let Err(error) = sync.request_body(message).await {
                tracing::warn!(message = message.get(), %error, "cannot fetch that body");
            }
        });
    });
}

/// The choices about *this installation* that outlive a failed start.
///
/// Named for what it holds rather than for when it runs: `Startup` in this
/// module is already the enum that decides between an account and onboarding.
///
/// Held so a retry can rebuild everything the first attempt could not: which
/// keyring, which folder is the archive, how hard to sync. None of it depends
/// on the store, which is exactly why it survives the store not opening.
pub struct Installation {
    /// The one keyring this installation reads every credential from.
    pub secrets: std::sync::Arc<dyn postio_account::secret::SecretStore>,
    /// What the user is looking at, as the handlers see it.
    pub state: SharedState,
    /// `[mailboxes]`: which folder this server calls its archive.
    pub mailbox_roles: postio_model::RoleOverrides,
    /// `[sync]`: how hard to sync, and what to notify about.
    pub sync_config: postio_config::SyncConfig,
    /// `[storage] max_bytes`, or `None` for the documented default of
    /// unbounded. Read here beside the other two sections, and for the same
    /// reason: the sweep that reads it runs before there is anywhere else to
    /// have put it.
    pub storage_ceiling: Option<u64>,
    /// An open of the store started before there was a window to report
    /// to, waiting for the first [`open_the_store`] to take it (#1604). See
    /// [`Installation::start_opening`].
    early: std::cell::RefCell<Option<async_channel::Receiver<Progress>>>,
}

impl Installation {
    /// Start opening the store now, before any window exists.
    ///
    /// The open -- the keyring over D-Bus, then the engine's open of an
    /// encrypted file, then the search index -- needs nothing from the
    /// window, and waited for it anyway because it was started from
    /// `activate`, after the window was built and presented. Started here it
    /// overlaps the whole GTK start-up; the first [`open_the_store`] takes it
    /// over and drives it exactly as it would its own. A second call does
    /// nothing, and a retry after a refusal opens afresh as it always did.
    pub fn start_opening(&self) {
        let mut early = self.early.borrow_mut();
        if early.is_none() {
            *early = Some(open_the_store_on_a_thread(self.secrets.clone()));
        }
    }

    /// An installation with `secrets` and this build's defaults for
    /// everything `config.toml` would otherwise supply.
    ///
    /// [`run`] fills those in from the file. This is for whoever is driving
    /// the composition root without one — the tests that exist because
    /// `postio-bl2` was eight capabilities wired to nothing, and the only way
    /// to find out was to launch the binary.
    pub fn new(secrets: std::sync::Arc<dyn postio_account::secret::SecretStore>) -> Self {
        Installation {
            secrets,
            state: SharedState::default(),
            mailbox_roles: Default::default(),
            sync_config: Default::default(),
            storage_ceiling: None,
            early: std::cell::RefCell::new(None),
        }
    }
}

/// Everything downstream of the store key.
///
/// Built in one go because it is one dependency chain — the store feeds the
/// command bus, the bus feeds the runtime, the runtime feeds the wiring — and
/// a half-built one is not a state anything downstream knows how to handle.
/// Either the mail opens or a screen says why.
pub struct Opened {
    /// Everything the window and its panes read through.
    pub wiring: Wiring,
    /// What the bus answers, asked before it was handed over: the window's
    /// action seam carries *every* gesture, and the ones another consumer
    /// owns must not come back as "not wired up in this build".
    pub wired: Vec<postio_core::CommandId>,
    /// Taken on the first `activate`. `EventStream` is not `Clone` — there is
    /// one queue and exactly one reader of it — and `activate` can fire again
    /// when a second launch raises the window.
    pub events: Rc<std::cell::RefCell<Option<EventStream>>>,
    /// The store's owner, and the runtime every read is polled on. Held to
    /// the end of `run`, which is what stops it.
    pub host: postio_host::Host,
}

/// What the opening thread has to say, in the order it says it.
///
/// One channel rather than two, so the sentence on screen and the answer
/// cannot arrive out of order — a `Stage` delivered after `Done` would put a
/// plate over a window that already has its mail in it.
enum Progress {
    /// What is being waited on now, for the window to say so if the wait
    /// outlasts the threshold.
    Stage(postio_ui::list_state::Waiting),
    /// The store, or the sentence explaining why there is not one.
    Done(Result<(Store, postio_storage::BlobStore), String>),
}

/// Read the keyring and open the store, on a thread, reporting as it goes.
///
/// **Not the runtime**, and not because one is unavailable — at this point in
/// startup there genuinely is none, since `Bridge` is built out of the store
/// this is opening. The retry on the `Unavailable` screen has made the same
/// call on the same kind of plain thread since #404, for the same reason.
///
/// Everything sent back is `Send`: a `Store` is a pool behind an `Arc`, a
/// `BlobStore` is a directory and some keys. The half of the old `open_with`
/// that is *not* — the command bus, the event hub, the `Wiring` the window
/// holds — is assembled on the main thread by [`assemble`] once this lands,
/// and costs nothing measurable next to the I/O this does.
fn open_the_store_on_a_thread(
    secrets: std::sync::Arc<dyn postio_account::secret::SecretStore>,
) -> async_channel::Receiver<Progress> {
    use postio_ui::list_state::Waiting;

    // Unbounded, and it matters: a bounded sender would block this thread on
    // a main loop that is busy drawing, which is the one thing the whole
    // arrangement exists to avoid.
    let (sender, receiver) = async_channel::unbounded();
    std::thread::spawn(move || {
        // A runtime of its own on this thread: opening the store is async
        // now, and this thread exists precisely so the main loop is free to
        // draw while it happens.
        //
        // Multi-threaded with one worker rather than `current_thread`, which
        // is the shape this wants: a `current_thread` runtime refuses
        // `block_in_place`, so any synchronous store read reached from inside
        // it aborts the process. One worker keeps the cost to a thread and
        // the invariant to one sentence -- every runtime here is
        // multi-threaded.
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = sender.send_blocking(Progress::Done(Err(format!(
                    "Postio could not start the worker that opens its store: {error}"
                ))));
                return;
            }
        };
        // The keyring first, and it is the wait least under Postio's
        // control: a D-Bus round trip to a service that may be showing a
        // passphrase prompt of its own, behind another window. 28 seconds,
        // once, on the live install.
        let _ = sender.send_blocking(Progress::Stage(Waiting::Keyring));
        let key = match postio_session::store_key_blocking(secrets.as_ref()) {
            Ok(key) => key,
            Err(error) => {
                let _ = sender.send_blocking(Progress::Done(Err(error.to_string())));
                return;
            }
        };
        let report = |stage| {
            let _ = sender.send_blocking(Progress::Stage(match stage {
                postio_session::Opening::Store => Waiting::Store,
                postio_session::Opening::Migrating => Waiting::Migrating,
                postio_session::Opening::Indexing => Waiting::Indexing,
            }));
        };
        let opened = runtime.block_on(postio_session::open_store_reporting(&key, &report));
        let _ = sender.send_blocking(Progress::Done(opened));
    });
    receiver
}

/// Open the store behind a window that is already on screen, and then feed it.
///
/// The whole of #1114's startup, in one place: the window says what it is
/// waiting on while this runs, says nothing at all if it finishes quickly
/// enough, and either fills with mail or is replaced by the screen that says
/// why it could not be.
///
/// `pub` for the reason [`present`] is: this is what `activate` calls, and a
/// composition root reachable only by launching the binary is what
/// `postio-bl2` cost. The one line a test still cannot reach is the call
/// itself, inside `run`'s `connect_activate`.
pub fn open_the_store(
    window: &Window,
    opened: &Rc<std::cell::RefCell<Option<Opened>>>,
    context: &Rc<Installation>,
    fed: &Rc<std::cell::Cell<bool>>,
    timeline: &Timeline,
) {
    // The open `run` started before the window existed, if it did; one of
    // our own otherwise (#1604).
    let early = context.early.borrow_mut().take();
    let progress = early.unwrap_or_else(|| open_the_store_on_a_thread(context.secrets.clone()));
    glib::spawn_future_local({
        let window = window.clone();
        let opened = Rc::clone(opened);
        let context = Rc::clone(context);
        let fed = Rc::clone(fed);
        let timeline = timeline.clone();
        async move {
            let mut answer = Err(
                // The thread went away without answering, which is a bug
                // rather than a condition — but the screen still has to say
                // something a person can act on.
                "Postio stopped opening its local store before it answered.".to_owned(),
            );
            while let Ok(progress) = progress.recv().await {
                match progress {
                    Progress::Stage(waiting) => window.set_waiting_on(waiting),
                    Progress::Done(done) => {
                        answer = done;
                        break;
                    }
                }
            }
            // Marked whether or not the store opened: a refused keyring still
            // spent the time, and a phase that only appears on the happy path
            // measures the wrong startup.
            timeline.mark(Phase::Store);

            let refused =
                match answer.and_then(|(database, blobs)| assemble(database, blobs, &context)) {
                    Ok(ready) => {
                        *opened.borrow_mut() = Some(ready);
                        None
                    }
                    Err(reason) => {
                        // Safe verbatim: no `SecretError` carries key material.
                        // The same sentence goes to the log and to the screen,
                        // because the log is for a bug report and the screen is
                        // for the person who has to unlock their keyring.
                        tracing::error!(reason, "the store did not open");
                        Some(reason)
                    }
                };
            // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
            // network work it reaches is spawned onto the runtime and answers over a
            // channel -- `onboarding::probe_with_offer` is the shape -- and what is
            // left is store reads, whose futures this engine makes self-contained.
            // Measured rather than assumed: `app_suite::glib_main_context` opens a
            // store and reads it on this context with no runtime anywhere, and fails
            // loudly if that stops being true.
            present(&window, &opened, &context, refused, &fed).await;
        }
    });
}

/// Start the store's owner over a store that is already open, and connect
/// the window to it as a client.
///
/// The half of startup that is main-thread work rather than I/O, split out
/// when the other half moved to a thread (#1114): none of it is slow -- the
/// cost this function has is the cost of starting a tokio runtime, which is
/// microseconds beside a schema migration.
fn assemble(
    database: Store,
    blobs: postio_storage::BlobStore,
    context: &Installation,
) -> Result<Opened, String> {
    // The store's owner (ADR 0041): its runtime, its event hub, its engines'
    // slot and a dispatcher per frontend. In this process until the desktop
    // reaches a daemon over the socket; the window is its client either way.
    let host = postio_host::Host::start(database, blobs, |wiring| {
        wiring
            .with_mailbox_roles(context.mailbox_roles.clone())
            .with_backfill(postio_session::backfill_policy(&context.sync_config))
            .with_watch(postio_session::watch_policy(&context.sync_config))
            .with_storage_ceiling(context.storage_ceiling)
            .with_secrets(context.secrets.clone())
    })
    .map_err(|error| {
        tracing::error!(error, "no runtime, so no mail: {error}");
        error
    })?;
    // Aimed with the window's own state, which the host is sent a snapshot
    // of with every command -- the selection a verb acts on is the one on
    // this screen, not another frontend's.
    let client = host
        .connect(postio_client::protocol::ClientKind::Gtk)
        .with_state(context.state.clone());
    let runtime = host.wiring().runtime.clone();

    // The window's commands: sent as they always were, through a
    // `CommandSender`, and passed to the client in the order they came.
    let (commands, queued) = postio_core::bridge::command_channel();
    runtime.spawn({
        let client = client.clone();
        async move {
            while let Some(command) = queued.recv().await {
                if client.send(command).await.is_err() {
                    return;
                }
            }
        }
    });
    // And what it hears: everybody's news, and what its own commands said
    // about themselves, which only this client is told.
    let (sink, stream) = postio_core::bridge::event_channel();
    let arriving = client.events();
    runtime.spawn(async move {
        while let Ok(envelope) = arriving.recv().await {
            if !sink.emit(envelope.event) {
                return;
            }
        }
    });

    Ok(Opened {
        wiring: Wiring {
            commands,
            ..host.wiring().clone()
        },
        wired: host.wired(),
        events: Rc::new(std::cell::RefCell::new(Some(stream))),
        host,
    })
}

/// Puts either the mail or the reason there is none in front of the user.
///
/// Called once the store has answered — on `activate` since #1114, after the
/// window is already up — and again by the retry on the screen below, which
/// is why it is a function rather than the body of a closure.
///
/// `pub` so a test can drive the state that only exists because the window
/// now comes first: a refusal arriving at a window somebody is already
/// looking at, and a retry from it (#1114). `postio-bl2` is the bead for what
/// a composition root only reachable by launching the binary costs.
pub async fn present(
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
        // The mail is behind the window from here on, so every command that
        // reads or writes it becomes available -- to the palette, the cheat
        // sheet and the keyboard at once, because all three ask the registry
        // (#1114). Before this line the window offers the chrome and nothing
        // else, which is exactly what it can do.
        window.set_store_open(true);
        // Everything this needs is taken out of the cell and the borrow
        // dropped, for the reason the comment above already gives -- and the
        // await below makes it sharper, because a borrow held across a
        // suspension lasts as long as the future rather than as long as the
        // statement. All four are handles.
        let (wiring, wired, events, notifier) = {
            let held = opened.borrow();
            let ready = held.as_ref().expect("just checked");
            let notifier = notifications::Notifier::new(
                ready.wiring.database.clone(),
                ready.wiring.store.clone(),
                ready.wiring.runtime.clone(),
                context.sync_config.clone(),
            );
            (
                ready.wiring.clone(),
                ready.wired.clone(),
                Rc::clone(&ready.events),
                notifier,
            )
        };
        open_or_onboard(
            window,
            &wiring,
            context.state.clone(),
            wired,
            events,
            notifier,
            Rc::clone(fed),
        )
        .await;
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
    // Under the window's chrome, as onboarding is: a hard stop is exactly the
    // screen somebody wants to close, and bare content has no close button.
    window.set_content(Some(&postio_gtk::widgets::under_window_chrome(&screen)));
    screen.focus_retry();

    screen.connect_retry({
        let screen = screen.clone();
        let window = window.clone();
        let opened = Rc::clone(opened);
        let context = Rc::clone(context);
        let fed = Rc::clone(fed);
        move || {
            screen.set_busy(true);
            // The same threaded open the first attempt made, so a retry that
            // succeeds continues exactly as a normal start would — including
            // saying which wait it is on, since a retry after a keyring
            // prompt is precisely the case where one of them is long.
            let progress = open_the_store_on_a_thread(context.secrets.clone());
            glib::spawn_future_local({
                let screen = screen.clone();
                let window = window.clone();
                let opened = Rc::clone(&opened);
                let context = Rc::clone(&context);
                let fed = Rc::clone(&fed);
                async move {
                    let mut answer = Err(
                        "Postio stopped opening its local store before it answered.".to_owned(),
                    );
                    while let Ok(progress) = progress.recv().await {
                        match progress {
                            // Recorded but not drawn: the `Unavailable`
                            // screen has replaced the window's content, so
                            // the list pane is not on screen to say anything.
                            // It is set anyway, because a retry that succeeds
                            // hands the window straight back to the panes and
                            // the wait is then theirs to describe.
                            Progress::Stage(waiting) => window.set_waiting_on(waiting),
                            Progress::Done(done) => {
                                answer = done;
                                break;
                            }
                        }
                    }
                    screen.set_busy(false);
                    match answer.and_then(|(database, blobs)| assemble(database, blobs, &context)) {
                        Ok(ready) => {
                            tracing::info!("the store opened on a retry");
                            *opened.borrow_mut() = Some(ready);
                            // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
                            // network work it reaches is spawned onto the runtime and answers over a
                            // channel -- `onboarding::probe_with_offer` is the shape -- and what is
                            // left is store reads, whose futures this engine makes self-contained.
                            // Measured rather than assumed: `app_suite::glib_main_context` opens a
                            // store and reads it on this context with no runtime anywhere, and fails
                            // loudly if that stops being true.
                            present(&window, &opened, &context, None, &fed).await;
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

    // A screen with a retry button on it is a usable UI, and the one the
    // budget has to be measured against when there is no mail to reach:
    // ADR 0014 Q3 makes this a hard stop, so *this* is where a start that
    // cannot open its store ends. Without it the timeline would stay open
    // and report `startup incomplete` on a launch that finished, badly but
    // completely.
    window.report_usable();
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

/// Decide which of the two startup does: the store owner's rule
/// (`postio_host::startup::route`), which carries out pending removals
/// first and opens an account only when the keyring has its password.
///
/// Async because reading the keyring is: it must be polled on the engine
/// runtime and answered over a channel -- never awaited on the GTK main
/// context. `feed.rs` explains the rule.
pub async fn startup_route(
    database: &Store,
    secrets: &dyn postio_account::secret::SecretStore,
) -> Startup {
    match postio_host::startup::route(database, secrets).await {
        postio_client::protocol::StartupRoute::Ready(account) => Startup::Ready(account),
        postio_client::protocol::StartupRoute::Onboard(account) => Startup::Onboard(account),
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
    use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

    /// A store with one enabled account in it, and the key its credential
    /// would be filed under.
    async fn provisioned() -> (Store, AccountKey) {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let account = postio_storage::test_support::account(&connection).await;
        drop(connection);
        let key = AccountKey::new(account.address.address.clone());
        (database, key)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_account_with_its_password_is_opened() {
        let (database, key) = provisioned().await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn an_account_whose_password_never_landed_goes_back_to_onboarding() {
        // The bug this test exists for: onboarding wrote the row, the keyring
        // write failed, and every launch after that opened an account that
        // could not authenticate and could not be repaired.
        let (database, _) = provisioned().await;

        match startup_route(&database, &MemorySecretStore::new()).await {
            Startup::Onboard(Some(prefill)) => assert_eq!(
                prefill.address.address, "test@example.com",
                "the screen has to come back prefilled, not empty"
            ),
            other => panic!("a row with no credential is not an account to open: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_keyring_goes_back_to_onboarding_too() {
        // Not the same fault, and the same dead end: a credential that cannot
        // be read is a credential the account does not have. The store here
        // *has* the item; it just will not open.
        let (database, key) = provisioned().await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_password_is_no_password() {
        let (database, key) = provisioned().await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_deletion_account_is_reaped_before_startup_decides_anything() {
        // #464: "Remove" in the settings panel only marks the row, so
        // something has to actually delete it -- once, at the next launch,
        // before an engine could otherwise start against it.
        let (database, _key) = provisioned().await;
        let connection = database.connect().await.expect("a connection");
        let id = postio_storage::repository::AccountRepository::new(&connection)
            .list()
            .await
            .expect("list")[0]
            .id;
        postio_storage::repository::AccountRepository::new(&connection)
            .mark_pending_deletion(id)
            .await
            .expect("mark");
        drop(connection);

        assert!(
            matches!(
                startup_route(&database, &MemorySecretStore::new()).await,
                Startup::Onboard(None)
            ),
            "a pending-deletion account is not there to open or to prefill from"
        );

        let connection = database.connect().await.expect("a connection");
        assert!(
            postio_storage::repository::AccountRepository::new(&connection)
                .get(id)
                .await
                .expect("get")
                .is_none(),
            "startup_route must actually reap it, not merely skip past it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_installation_has_nothing_to_prefill_with() {
        let database = postio_storage::test_support::memory().await;

        assert!(matches!(
            startup_route(&database, &MemorySecretStore::new()).await,
            Startup::Onboard(None)
        ));
    }
}
