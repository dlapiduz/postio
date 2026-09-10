//! One gesture, at most one render (#1340, spec FR-058).
//!
//! #749's investigation found `reading.rs` wired the filler to **both**
//! `connect_cursor_moved` and `connect_activated`, and only the first
//! deduplicates. So `Enter` on the row the cursor was already on issued a
//! second complete document load for the same message. The call site called
//! it "harmless… costs a store read and nothing else"; it cost a full
//! document teardown and reload, which is what a person saw as the pane
//! flashing.
//!
//! **Every test underneath it stayed green.** The rows came back correct and
//! the store was asked once, so nothing that counted queries or inspected
//! widgets could see it. `postio_storage`'s counters cannot either — a
//! duplicate document load issues no query. `postio_ui::test_support`
//! exists for exactly this, and this is the assertion it was built for.
//!
//! Driven through the real composition root, because the fault was in the
//! wiring rather than in any widget in isolation.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
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

pub fn one_gesture_renders_once_and_reselecting_renders_nothing() {
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
    assert!(
        report.message_count > 1,
        "need at least two rows to move between"
    );
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
        "the Flagged view never filled"
    );

    assert!(
        settle_until(|| window.reading()),
        "the pane never filled for the autoselected row, so nothing below \
         can be attributed to a gesture"
    );

    // ── moving the cursor renders exactly once ───────────────────────────
    let before = postio_ui::test_support::renders_issued();
    let first = list.cursor_id().expect("the cursor is on a row");
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
    assert!(
        settle_until(|| list.cursor_id() != Some(first)),
        "`j` did not move the cursor, so this test cannot say what the pane did"
    );
    assert!(
        settle_until(|| postio_ui::test_support::renders_issued() > before),
        "the cursor moved to another message and nothing rendered"
    );
    // Let any second load that is coming actually arrive, or this asserts
    // that a race has not finished rather than that it cannot happen.
    for _ in 0..40 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let moved = postio_ui::test_support::renders_issued();
    assert_eq!(
        moved - before,
        1,
        "one keystroke, {} renders. A gesture that loads the same document \
         twice is #749: a full teardown and reload the user sees as a flash, \
         with every other test still green",
        moved - before
    );

    // ── and activating the row already under the cursor renders nothing ──
    //
    // The exact shape #749 found: `report_cursor` deduplicates, `activated`
    // did not, so Enter on the current row went straight through to a second
    // complete load of a document already on screen.
    window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
    for _ in 0..40 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        postio_ui::test_support::renders_issued(),
        moved,
        "Enter on the row the cursor was already on re-rendered a document \
         that was already on screen"
    );
}
