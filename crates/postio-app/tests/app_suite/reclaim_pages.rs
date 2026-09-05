//! The store's own pages are reclaimed by a running application (#381).
//!
//! `storage_suite/vacuum.rs` proves `Database::adopt_incremental_vacuum` and
//! `reclaim_free_pages` do what they say when something calls them. Nothing
//! in that suite can fail when nothing does — which is precisely the history
//! of the three blob sweeps in #416: written, tested, documented, and wired
//! to nobody, so deleting mail freed nothing for as long as they existed.
//!
//! So this asserts the reach: a store created before `auto_vacuum` was
//! chosen, handed to the same `feed_the_window` the binary calls, is
//! converted — by the application, on its own, with nobody in this test
//! naming the method.
//!
//! Nothing here touches the network: the store is local and the sync engine
//! is never started.

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
use postio_storage::{BlobStore, Database, test_support};

/// `auto_vacuum = INCREMENTAL`, as SQLite numbers the modes.
const INCREMENTAL: i64 = 2;

fn auto_vacuum(database: &Database) -> i64 {
    let connection = database.connection().expect("a connection");
    postio_storage::db::read_pragmas(&connection)
        .expect("the pragmas in force")
        .auto_vacuum
}

pub fn a_store_written_before_the_setting_is_converted_by_the_application() {
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

    let directory = tempfile::tempdir().expect("a store directory");
    let path = directory.path().join("store.db");

    // A store as it would have been written before #381: keyed and migrated
    // with `auto_vacuum` at SQLite's own default of NONE.
    let database = test_support::unconverted_store(&path);
    seed_small(&database, 11);
    assert_eq!(
        auto_vacuum(&database),
        0,
        "the fixture was supposed to be an unconverted store, so this test \
         could not have failed"
    );

    let blobs = BlobStore::open(
        directory.path().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
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

    // The same call `run` makes, and the only thing this test asks of the
    // application. Nothing below names the conversion.
    feed_the_window(&window, &wiring).expect("the seeded store has an account");

    assert!(
        settle_until(|| auto_vacuum(&database) == INCREMENTAL),
        "the application ran over a store that can never hand a freed page \
         back to the filesystem, and left it that way. The conversion exists \
         and is tested; nothing calls it -- see #416 for the last three times \
         that happened."
    );

    bridge.shutdown();
}
