//! Issue #602: the letters you cannot type into a reply.
//!
//! `e` in the composer's body ran *reply* instead of typing an `e`, so a
//! half-written reply answered itself. Every single-key binding did it — `a`,
//! `u`, `t` — which between them make the commonest letters in English
//! unavailable in the one place a person writes English.
//!
//! The rule the window already has is right: **typing always wins**, and
//! `Window::is_typing` decides whether it applies. What it asked was whether
//! the focused widget is a `GtkText` or a `GtkTextView`. The composer's body
//! is neither — it is a `WebView` over a `contenteditable` document — so the
//! rule never fired for the one field it mattered most in. `To` and `Subject`
//! were fine, because an entry hands the keyboard to the `GtkText` inside it,
//! which is why this looked intermittent rather than total.
//!
//! Widening the type test to "any `WebView`" would have been wrong: the
//! reader is a `WebView` too, and `e` must keep meaning reply while reading.
//! What separates them is that the body is editable, which
//! `Composer::focused_field` already knows.
//!
//! # What this asserts
//!
//! That the window leaves the key alone — `Propagation::Proceed` — rather than
//! that a character appeared in the document. The character's journey from
//! there is WebKit's and needs a JavaScript round trip; what broke here, and
//! what can break again, is the window deciding the key was a command.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` opens the socket
//! and this never calls it.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::composer::Field;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str) -> glib::Propagation {
    let outcome = window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    outcome
}

pub fn every_letter_can_be_typed_into_the_composer_body() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

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
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    settle();

    // ── `c` opens the composer, the way the canvas says ──────────────────
    press(&window, "c");
    let composer = window.composer();
    assert!(composer.is_open(), "`c` did not open the composer");

    // ── put the keyboard in the body, which is where writing happens ─────
    assert!(
        composer.test_focus_field(Field::Body),
        "the body would not take the keyboard"
    );
    settle();
    assert_eq!(
        composer.focused_field(),
        Some(Field::Body),
        "the body does not have the keyboard, so this test would pass for the \
         wrong reason"
    );

    // ── the letters that were commands ───────────────────────────────────
    for key in ["e", "a", "u", "t"] {
        assert_eq!(
            press(&window, key),
            glib::Propagation::Proceed,
            "`{key}` was taken as a command while the cursor was in the body \
             of a message being written. Typing always wins over a single-key \
             binding, and the body is the one field that rule has to hold in."
        );
        assert!(
            composer.is_open(),
            "`{key}` did something to the composer instead of being typed \
             into it"
        );
    }

    bridge.shutdown();
}
