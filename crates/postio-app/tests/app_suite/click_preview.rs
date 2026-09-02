//! Issue #70, come back for the pointer.
//!
//! `0.1.0` shipped a blank reading pane; #70 fixed it for the keyboard by
//! making the preview follow the cursor. The pointer kept the bug. Every
//! path to the cursor except one goes through `move_cursor_to`, which sets
//! the `landed` flag that `report_cursor` gates on — and the one that does
//! not is an ordinary single click. `GtkListView` moved its own cursor,
//! `report_cursor` found `landed` still false, and returned without telling
//! the reader anything: no body, no header, no action bar, and not even one
//! of the `Absent` plates that #70 exists to show.
//!
//! It survived because nothing drove a click at the list. `cursor_preview`
//! and the rest of this suite move with `j`, which sets the flag, and the
//! assertions are about what the reader was *told* rather than what a person
//! would see. A mouse user got a blank pane for the whole session, until
//! they happened to press a key.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket and this never calls it. `seed_small` marks every message
//! `BodyState::NotFetched`, so the pane here fills with the "Downloading this
//! message" plate rather than a body — which is the right assertion anyway:
//! what #70 is about is the pane saying *something*.
//!
//! Runs in the Flagged view since #755, for `cursor_preview.rs`'s reason: a
//! folder row is a conversation now and opens the conversation pane, and
//! what this file constrains is the single-message reader answering a
//! click. The fixture flags every message so the view holds them all.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn clicking_a_message_fills_the_reading_pane() {
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
    assert!(report.message_count > 2, "need rows to click");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // Every message but the newest flagged, before anything is wired: the
    // Flagged view is where rows are genuinely single messages (see the
    // module comment). The newest is left out because the folder view the
    // window opens on has already reported it — its row *is* that message —
    // and the cursor's dedup would then swallow the Flagged view's own
    // first report, leaving the pane unfilled.
    let flagged_total: u32 = {
        let connection = database.connection().expect("a connection");
        connection
            .execute(
                "UPDATE messages SET flagged = 1 WHERE id NOT IN \
                 (SELECT id FROM messages ORDER BY received_at DESC LIMIT 1)",
                [],
            )
            .expect("the fixture writes");
        connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE flagged = 1",
                [],
                |row| row.get(0),
            )
            .expect("a count")
    };

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

    // ── the same call `run` makes ────────────────────────────────────────
    let wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");

    // Into the Flagged view, the way the sidebar's row would take it — but
    // only after the sidebar's own default pick has landed: the folder list
    // loads asynchronously and picking the default folder is what it does
    // on arrival, which would stomp a scope opened before it. Then wait for
    // the swap itself, because the model keeps the folder's rows until the
    // Flagged page answers.
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the opening folder never filled, so no scope can be left"
    );
    wired
        .feeds
        .messages
        .open(postio_model::ListScope::Flagged(report.account.id));
    assert!(
        settle_until(|| list.model().n_items() == flagged_total),
        "the Flagged view never filled, so there is nothing to click"
    );

    // The autoselect fills the pane now (#601), so `window.reading()` is no
    // longer what tells a click apart from a window that just opened. What
    // this test is about is the *click* reaching the reader, so it clears the
    // pane first and watches it come back.
    window.clear_reader();
    assert!(!window.reading(), "the pane was just cleared");

    // ── clicking the row the autoselect is already on ────────────────────
    // The first thing a mouse user does: open the top message. The cursor is
    // already there, so the position does not change and no
    // `notify::selected` is emitted — and the autoselect has already put that
    // id in `reported`. Both have to be got past for the pane to fill.
    assert_eq!(list.cursor().selected(), 0, "the autoselect lands on row 0");
    list.click_row(0);
    assert!(
        settle_until(|| window.reading()),
        "clicking the top message left the reading pane empty. The row is \
         under the cursor and the store has the message; what is missing is \
         the pane being told about it at all — not even an `Absent` plate. \
         That is #70 again, for the pointer: every other path to the cursor \
         goes through `move_cursor_to`, and a plain click is the one that \
         does not."
    );

    // ── and a click that does move the cursor ────────────────────────────
    let first = list.cursor_id().expect("the cursor is on a row");
    list.click_row(2);
    assert!(
        settle_until(|| window.reading()),
        "clicking a different row left the reading pane empty"
    );
    assert_ne!(
        list.cursor_id().expect("the cursor is on a row"),
        first,
        "the click did not move the cursor"
    );

    bridge.shutdown();
}
