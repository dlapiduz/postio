//! Opening the store gives back the disk nothing is using.
//!
//! Another instance of `postio-bl2`, and the reason this test is at the far
//! end rather than in `postio-storage`: `BlobStore::collect_garbage` and
//! `BlobStore::purge_temporary` were both written, tested and documented, and
//! **neither had a production caller** (#416). Their own unit tests passed the
//! whole time.
//!
//! `MessageRepository::delete` removes a message's row and deliberately does
//! not touch its blobs, because the schema delegates reclamation to that
//! sweep. With no caller, deleting mail freed nothing, ever — and a
//! `UIDVALIDITY` reset, which wipes and re-syncs a whole mailbox, orphaned
//! every blob in it at once.
//!
//! So the assertion is not "the sweep works". `postio-storage` proves that
//! itself, and proved it while the bug was live. It is **"a store this
//! application opened has had its orphans reclaimed"**, which is the sentence
//! that was false.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set before the app under test starts, which is the one
// moment it is sound -- the same reasoning `wiring.rs` records.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

use postio_gtk::{app, fonts, style};

pub fn opening_a_store_reclaims_what_nothing_references() {
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

    // What a deleted message leaves behind: bytes on disk that no row names.
    // Written directly rather than by deleting a seeded message, so the test
    // states the condition it is about instead of depending on which columns
    // the fixture happens to fill.
    let orphan = blobs
        .put(b"the body of a message that is no longer in the database")
        .expect("put an orphan");
    // And what a crash mid-fetch leaves: a part file nothing will ever finish.
    let debris = blobs.temporary_directory().join("9999-0.part");
    std::fs::write(&debris, b"half a message").expect("stage some debris");

    assert!(blobs.contains(&orphan), "the orphan is there to begin with");

    // Backdated past `BLOB_GRACE_PERIOD`, rather than shortening the grace
    // period for the test.
    //
    // The first version of this test wrote the orphan and expected it gone,
    // and it failed -- because production sweeps with a one-hour grace period
    // and the blob was seconds old. That failure is the grace period working:
    // a blob is written before the row referencing it is committed, so inside
    // that window a healthy blob is indistinguishable from an orphan.
    //
    // Injecting a shorter period would have made the test pass while testing
    // a configuration that never ships. Ageing the file exercises the real
    // constant, and the real case: a blob orphaned an hour ago.
    let aged = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    let path = blobs.path_of(&orphan).expect("the orphan's path");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open the orphan")
        .set_times(std::fs::FileTimes::new().set_modified(aged))
        .expect("age the orphan");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database,
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // The same call `run` makes.
    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    let _ = feeds;

    assert!(
        settle_until(|| !blobs.contains(&orphan)),
        "opening the store left a blob nothing references on disk"
    );
    assert!(
        settle_until(|| !debris.exists()),
        "opening the store left debris from a fetch that never finished"
    );
}
