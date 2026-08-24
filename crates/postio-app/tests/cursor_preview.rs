//! Issue #70, both causes, in the running application.
//!
//! `0.1.0` shipped a mail client whose right-hand column was blank. Two
//! independent faults produced the identical symptom, and this file exists
//! because each one passed every test underneath it:
//!
//! * **Cause B** — the pane was fed by `connect_activated`, which is Enter or
//!   a double click. Moving the cursor with `j` fed it nothing, so the pane
//!   showed whatever was last opened, or on a fresh window nothing at all.
//!   The maintainer settled the design: the preview follows the cursor.
//! * **Cause A** — a message whose body has not been downloaded produced an
//!   empty `MessageBody`, which rendered as an empty pane. Silently: no
//!   status, no error, no log line. With the pane following the cursor this
//!   stops being an edge case and becomes the *ordinary* case, because a
//!   mailbox backfills over minutes and the cursor moves in milliseconds.
//!
//! The assertions are deliberately about the **application**, not the
//! widgets: `j` through the real keymap, and the pane's own answer about what
//! it is showing. `gtk_cursor_preview.rs` covers the signal and
//! `reader::view`'s unit tests cover the words; only a test at this level can
//! fail when nothing joins them up.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket and this never calls it. `seed_small` marks every message
//! `BodyState::NotFetched`, which is exactly Cause A's condition — so the
//! partial state here is the real one, not a simulated one.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::reader::Absent;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// `j`, through the keymap the application actually runs.
fn press_j(window: &Window) {
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
}

#[test]
fn the_pane_follows_the_cursor_and_says_why_a_body_is_missing() {
    let state_dir = std::env::temp_dir().join(format!("postio-cursor-{}", std::process::id()));
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
    assert!(
        report.message_count > 1,
        "need at least two rows to move between"
    );
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ────────────────────────────────────────
    let wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let _ = &wired;

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the list is empty, so there is no cursor to move"
    );

    // ── an untouched window keeps the pane's empty state ─────────────────
    // The cursor is autoselected onto row 0 the moment the list has rows,
    // and that is not somebody reading. `reading.rs`'s own test asserts the
    // same thing; filling here would also, once #71's dwell timer lands,
    // mark the newest message read merely because Postio was opened.
    assert!(
        !window.reading(),
        "an untouched window shows the pane's empty state, not a message"
    );

    // ── `j` moves the cursor, and the pane follows it ────────────────────
    let first = list.cursor_id().expect("the cursor is on a row");
    press_j(&window);
    assert!(
        settle_until(|| list.cursor_id() != Some(first)),
        "`j` did not move the cursor, so this test cannot say anything \
         about what the pane did"
    );
    let second = list.cursor_id().expect("the cursor is on a row");
    assert_ne!(first, second, "the cursor should be on a different message");

    // ── Cause B: moving the cursor fills the pane, with no Return ────────
    // On `0.1.0` this column stayed blank until the user found out that
    // Return was required, which is what made a working mail client look
    // like a broken one.
    assert!(
        settle_until(|| window.reading()),
        "the cursor moved to another message and the reading pane never \
         filled. Nothing feeds the reader from the cursor."
    );
    assert!(
        window.reader().widget().is_visible(),
        "the pane says it is reading and the reader is not on screen"
    );

    // ── Cause A: and it says why there is no body ────────────────────────
    // Every seeded message is `BodyState::NotFetched`, so there is nothing
    // on this machine to draw. The pane must say which kind of nothing that
    // is rather than render blank -- the bug was that "still downloading"
    // and "broken" were the same picture.
    assert_eq!(
        window.reader().absent(),
        Some(Absent::Partial),
        "a message with no downloaded body must say so, not render blank"
    );

    // ── and none of it dialled anything ──────────────────────────────────
    // No engine was started. The pane filled from the local store alone,
    // which is what makes the partial state honest rather than a spinner
    // waiting on a socket.
}
