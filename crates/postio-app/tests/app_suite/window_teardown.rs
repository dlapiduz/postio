//! A window wired by the composition root still frees when it is destroyed
//! (#1072).
//!
//! `gtk_window_teardown.rs` asserts this for a bare `Window` and its reader,
//! and could not see this leak: it needed `feed_the_window`, which a
//! `postio-gtk` test cannot call. The cycle is the one #794 catalogued three
//! times over — a handler stored on a child widget holding a strong
//! reference back to the window that owns it. #794 and an earlier pass on
//! this issue made nine such captures in `postio-app`'s composition wiring
//! weak without moving this test; what was left standing was the tenth, one
//! layer down in `postio-gtk::window::Window::composer` itself —
//! `connect_opened`/`connect_closed` on the composer it lazily builds,
//! registered the moment anything (here, `postio-app::compose::install`)
//! asks for a composer at all. The rule against it is written in the same
//! file the leak was in: *"Weak, because the window owns the finder that
//! owns this handler; a strong clone here is a cycle that keeps the window
//! alive for the life of the process."*
//!
//! # Why it is worth a test rather than just a fix
//!
//! Because nothing else can notice. Postio opens one window for the life of
//! the process, so the leak is invisible in the application; what it costs is
//! `app_suite` and `gtk_suite`, which build and destroy a window per case
//! across roughly 350 cases in one binary, and whose peak memory therefore
//! grows with every case added. A leak nobody can see is one that comes back.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn a_window_the_composition_root_wired_still_frees_when_destroyed() {
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
    seed_small(&database, 11);
    ensure_search_index(&database).expect("the index is part of opening the store");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let weak = {
        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}

        // The wiring is the point: `search::install` is what registers the
        // handlers on the finder, so a bare `Window` cannot show this.
        feed_the_window(&window, &wiring).expect("the seeded store has an account");

        let weak = window.downgrade();
        window.destroy();
        weak
    };
    settle();

    assert!(
        weak.upgrade().is_none(),
        "a window that ran through `feed_the_window` outlived its own \
         destruction. Something registered on a child widget is holding a \
         strong reference back to it -- the cycle #794 catalogued, and the \
         one `search.rs` states the rule against at `install_run`"
    );

    bridge.shutdown();
    drop(directory);
    drop(state_dir);
}
