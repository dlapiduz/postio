//! The window shows stored mail before it talks to a server (#1434).
//!
//! `open_account` used to call `start_syncing` and *then*
//! `feed_the_window`, so opening an account connected, authenticated and
//! listed the server's folders before one stored message reached the list.
//! Measured on a real account, the first frame was 2282ms of a 2474ms
//! startup against a 500ms budget, and the log named what it waited for:
//!
//!     56.558  opening account
//!     57.085  connected and authenticated      <- a round trip
//!     57.258  listed the server's folders      <- another
//!     58.713  first frame
//!
//! `docs/PRODUCT.md` §18 budgets startup at 500ms, and the architecture note
//! says the UI never awaits the network. This was the one place it did.
//!
//! **The mail is already on disk.** A seeded store, a window, no server that
//! could possibly answer -- and the list must fill anyway.
//!
//! Skips without a display. Nothing here touches the network, which is the
//! point: the address below is unroutable, so a build that waits for a
//! connection waits for this test's timeout instead of passing.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. This sets it before the app under test starts,
// which is the one moment it is sound.

use crate::{settle, settle_until};
use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::BlobStore;
use postio_storage::seed::seed_small;
use postio_storage::test_support;

pub fn the_list_fills_from_storage_without_a_server() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test, before the app runs.
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
    settle();

    let wired = feed_the_window(&window, &wiring);
    assert!(
        wired.is_some(),
        "the seeded store has an account to feed from"
    );

    // The whole assertion: rows, with nothing connected and nothing to
    // connect to. If reading stored mail ever needs the network again, this
    // is where it stops.
    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "the list never filled from a seeded store. Stored mail must reach \
         the screen without a server -- the messages are already on disk, \
         and startup awaited two IMAP round trips before drawing them (#1434)"
    );

    window.destroy();
}
