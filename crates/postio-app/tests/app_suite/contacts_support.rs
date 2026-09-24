//! A wired window over a store, with a real bus, for the Contacts cases:
//! the keys have to reach the store and the store's answer the screen.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::BlobStore;

/// The window and what keeps it running. Hold `_blobs` for the test's life.
pub struct App {
    pub window: Window,
    pub bridge: Bridge,
    _blobs: tempfile::TempDir,
}

/// Fonts and styles, or `false` when there is no display to test on.
pub fn display() -> bool {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    true
}

/// `database` in a window the way the binary composes one: fed, commands
/// installed on a real bus, and both event streams drained onto the panes.
pub async fn wire(database: &postio_storage::Store) -> App {
    let blobs_dir = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        blobs_dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let state = postio_core::state::SharedState::default();
    let bus = postio_app::actions::wire(
        Dispatcher::builder(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
    let (bridge, replies) = Bridge::new(bus).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );
    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}
    let wired = feed_the_window(&window, &wiring)
        .await
        .expect("the store has an account");
    let feeds = wired.feeds.clone();
    postio_app::commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        bus_verbs,
    );
    let notifier = postio_app::notifications::Notifier::new(
        database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    for stream in [events, replies] {
        postio_app::commands::drain(&window, &feeds, stream, notifier.clone(), state.clone());
    }
    App {
        window,
        bridge,
        _blobs: blobs_dir,
    }
}

/// A key, as a person presses it.
pub fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    crate::settle();
}

/// The names the Contacts list draws, as far as its rows have arrived.
pub fn drawn(window: &Window) -> Vec<String> {
    let model = window.contacts().model();
    (0..model.n_items())
        .filter_map(|position| model.row(position).map(|row| row.name))
        .collect()
}
