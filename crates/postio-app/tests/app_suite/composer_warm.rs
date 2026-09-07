//! The editing surface is started before anybody asks to compose.
//!
//! `Composer::warm` exists and is proved by `postio-gtk`'s
//! `gtk_composer_warm`. What that cannot prove is that anything ever calls it,
//! which is the whole of #327 and `postio-bl2`: a capability implemented,
//! tested, documented and wired to nothing, with a green suite throughout.
//!
//! So this asserts the wiring, from where the application starts: a real store,
//! a real `Window`, and `feed_the_window` — the same call `run` makes. Nobody
//! opens the composer. It has to be warm anyway, and the reading pane has to
//! still be the reading pane.
//!
//! Why it matters: the composer's surface is ADR 0003's `WebView`, and its
//! first load starts a WebKit web process. Measured, splitting `Composer::open`
//! into its parts, the first open cost 28.7ms against 0.2ms for every one after
//! — all of it falling on the first message a person sits down to write (#1216).
//!
//! Nothing here touches the network: `feed_the_window` reads the local store,
//! `start_syncing` is the half that opens a socket and is never called, and the
//! editor's `WebView` has network off.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn the_window_warms_its_editing_surface_without_being_asked() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    assert!(
        !window.composer().is_warm(),
        "the composer was warm before the window was fed, so this test could \
         not fail"
    );

    // The same call `run` makes, and then nothing but time passing.
    let wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let warmed = settle_until(|| window.composer().is_warm());

    assert!(
        warmed,
        "nothing warmed the editing surface. `Composer::warm` exists and its \
         own test passes; that is exactly the shape of bug #327 is about — \
         check what is supposed to *call* it."
    );
    assert!(
        !window.composer().is_open(),
        "warming the editing surface opened the composer over the reading pane"
    );

    let _ = wired;
    bridge.shutdown();
}
