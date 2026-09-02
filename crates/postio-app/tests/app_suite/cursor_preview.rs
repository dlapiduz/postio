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
//! # Why it runs in the Flagged view
//!
//! Since #755, a folder row stands for a conversation and landing on it
//! opens the conversation pane — `conversation_by_default.rs` is that test.
//! What *this* file constrains is the single-message reader following the
//! cursor, and a query view is where single-message rows genuinely live:
//! `Row::is_thread()` is false there by construction. The fixture flags
//! every message so the Flagged view holds them all.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::reader::Absent;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// `j`, through the keymap the application actually runs.
fn press_j(window: &Window) {
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
}

pub fn the_pane_follows_the_cursor_and_says_why_a_body_is_missing() {
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
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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

    // ── an untouched list shows the row it selected ──────────────────────
    // The cursor is autoselected onto row 0 the moment the list has rows.
    // That is not somebody *choosing* a message, but the pane fills for it
    // all the same (#601): a window that opens with a row selected and
    // nothing beside it reads as a broken app, which is #70's complaint
    // wearing a different hat.
    //
    // What the autoselect must not do is start #71's dwell clock — that
    // would mark the newest message read for no reason but that Postio was
    // opened. `dwell_wiring` is where that is asserted, over the real timer.
    assert!(
        settle_until(|| window.reading()),
        "the view opened with a row under the cursor and an empty pane \
         beside it"
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
    //
    // `Offline`, not `Partial`: no engine was ever started, so `Folders`'
    // connection tracker is still at its own starting answer -- "offline,
    // never synced" -- and the pane has to say that honestly rather than
    // promise a backfill nothing here is running (issue #117;
    // `reading_offline.rs` covers the online and reconnecting cases this
    // test does not touch).
    assert_eq!(
        window.reader().absent(),
        Some(Absent::Offline),
        "a message with no downloaded body and no engine ever started must \
         say so, not promise a backfill that cannot run"
    );

    // ── and none of it dialled anything ──────────────────────────────────
    // No engine was started. The pane filled from the local store alone,
    // which is what makes the offline state honest rather than a spinner
    // waiting on a socket.
}
