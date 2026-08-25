//! Can a person actually detach the composer in the running application?
//!
//! `crates/postio-gtk/tests/gtk_composer_detach.rs` proves the widget does the
//! right thing when something asks it to. That is not the same claim, and the
//! difference is the whole of `postio-bl2`: the search UI was built, tested
//! and fed by nothing, the parts panel had no command to open it, and the
//! Reader was never mounted at all — a mail client that could not read mail,
//! with every test green.
//!
//! So this starts where the application starts — a seeded store, a real
//! `Window`, and `feed_the_window`, the same call `run` makes — and asserts
//! from the far end: press the keys a person would press, and a second window
//! exists with the composition in it. Nothing in here reaches for the
//! composer to ask it something; the keystroke has to travel the registry,
//! the keymap, the resolver, `Window::act` and the composer's own dispatch on
//! its own, exactly as it does on a desktop.
//!
//! Nothing here touches the network: `start_syncing` is never called.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::Context;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// A key press into the main window. GTK4 gives no supported way to
/// synthesize a GDK event, so this is the same call the window's own
/// controller makes — one line below where a real key would land.
fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

#[test]
fn the_detach_key_reaches_the_composer_in_a_wired_application() {
    let state_dir = std::env::temp_dir().join(format!("postio-detach-app-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();

    // ── the same call `run` makes: this is what mounts the composer ──────
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    settle();

    let composer = window.composer();
    assert!(!composer.is_open(), "nothing is being composed yet");

    // ── `c` — the key on the design canvas, not a method call ────────────
    press(&window, "c", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`c` did not reach the composer, so this test cannot say anything \
         about detaching"
    );
    assert_eq!(window.context(), Context::Composer);
    assert!(!composer.is_detached(), "in-place is the default");

    // ── `ctrl+shift+o` — the whole point ─────────────────────────────────
    press(
        &window,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );

    let host = composer.detached_window().expect(
        "the key resolved to nothing a person could see. Every layer under \
         this one passes; that is the shape of bug postio-bl2 is about — \
         check what is *between* the registry and the composer.",
    );
    assert!(composer.is_detached());
    assert!(host.is_visible(), "a window nobody can see is not a window");
    assert!(!host.is_modal(), "the main window must stay usable");
    assert_eq!(
        window.context(),
        Context::List,
        "and the main window got its keyboard back, so mail can still be read"
    );

    // ── the key in the detached window puts it back, through the same
    //    registry: one command, two containers ───────────────────────────
    composer.handle_key(
        gdk::Key::from_name("o").unwrap(),
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    settle();

    assert!(!composer.is_detached(), "and it came home");
    assert!(composer.is_open(), "still one composition, still open");
    assert_eq!(window.context(), Context::Composer);

    bridge.shutdown();
}
