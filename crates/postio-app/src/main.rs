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

mod feed;
mod paths;

use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::startup::{Phase, Timeline};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_runtime::store::{MailStore, SqliteStore};
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

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

    // The runtime. Nothing is dispatched through it yet — the command bus is
    // still `postio-agr`'s — but it owns the tokio threads every read is
    // polled on, so it has to exist before the first one.
    let runtime = match Bridge::new(handler_fn(|_, _| async {})) {
        Ok((bridge, _events)) => Some(bridge),
        Err(error) => {
            eprintln!("postio: no runtime, so no mail: {error}");
            None
        }
    };

    let application = app::build_with(timeline);

    // Connected *after* the frontend's own handler, so the window it makes is
    // already there to be fed. Signal handlers run in the order they were
    // connected, which is the whole of the arrangement.
    let wiring = runtime.as_ref().and_then(open_store);
    application.connect_activate(move |application| {
        let Some(window) = application.active_window().and_downcast::<Window>() else {
            return;
        };
        if let Some(wiring) = &wiring {
            feed_the_window(&window, wiring);
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
    store: Arc<dyn MailStore>,
    runtime: tokio::runtime::Handle,
}

/// Open the local store, or explain why there is none.
///
/// A missing or unreadable database is not a reason to refuse to start: the
/// window opens, says it has never synced, and stays usable for everything
/// that does not need mail. A mail client that will not open is worse than one
/// with nothing in it.
fn open_store(bridge: &Bridge) -> Option<Wiring> {
    let path = paths::store_path();
    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("postio: cannot open {}: {error}", path.display());
            return None;
        }
    };
    Some(Wiring {
        store: Arc::new(SqliteStore::new(&database)),
        database,
        runtime: bridge.handle(),
    })
}

/// Point the window's panes at the store.
///
/// Silent when there is no account yet: the sidebar already says what is true
/// — offline, never synced, no folders — and inventing an account to fill it
/// would be worse than an empty one. `postio-hiy` is the screen that creates
/// the first one.
fn feed_the_window(window: &Window, wiring: &Wiring) {
    let Some(account) = first_account(&wiring.database) else {
        return;
    };
    let sources = feed::Sources::new(wiring.store.clone(), wiring.runtime.clone());
    // Leaked deliberately: the feeds live as long as the window, the window
    // lives as long as the process, and a handle threaded through `activate`
    // only to be dropped at exit would be ceremony for a process that is
    // ending anyway.
    Box::leak(Box::new(window.install_feeds(
        account.id,
        account.address.address.as_str(),
        sources.clone(),
        sources,
    )));
}

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
