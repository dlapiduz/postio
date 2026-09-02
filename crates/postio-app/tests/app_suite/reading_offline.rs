//! Issue #117: the reading pane's wait has to say which wait it is.
//!
//! `load_body_or_reason` classified a missing body from what the store
//! knew — `BodyState` and the blob rows — and neither of those says whether
//! the engine is *connected*. `ConnectionState` never reached `postio-app`
//! at all, so every message with no local body said "downloading", offline
//! included: true, and not the sentence that would actually explain the
//! wait. This proves the three things `reading.rs`'s doc now promises:
//! offline shows `Absent::Offline`, connecting flips it to the ordinary
//! `Absent::Partial`, and losing the connection again repaints a pane that
//! is already open rather than leaving stale words on screen until the
//! cursor happens to move.
//!
//! # Simulated connectivity, no socket opened
//!
//! `feed_the_window` reads the local store; nothing here calls
//! `start_syncing`. Connectivity is driven the way `gtk_feeds.rs` drives it
//! — `Feeds::apply` with a `ConnectionChanged` event — which is exactly what
//! a real engine's own connection tracker feeds into the same seam.
//!
//! Runs in the Flagged view since #755, for `cursor_preview.rs`'s reason: a
//! folder row is a conversation now and opens the conversation pane, and
//! the `Absent` plates this proves live on the single-message reader. The
//! fixture flags every message but the newest so the view holds plenty.
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
use postio_core::{ConnectionState, Event};
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

pub fn the_pane_says_offline_and_updates_the_moment_the_connection_does() {
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
    let report = seed_small(&database, 21);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // Every message but the newest flagged, before anything is wired: the
    // Flagged view is where rows are genuinely single messages (see the
    // module comment), and leaving the newest out is what makes the swap to
    // it observable — the counts differ, and the first row is a message the
    // folder view has not already reported.
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
        "the Flagged view never filled, so there is no cursor to move"
    );
    press_j(&window);
    assert!(
        settle_until(|| window.reading()),
        "the cursor moved and the reading pane never filled"
    );

    // ── nothing has connected yet: the default is honestly offline ────────
    // `Folders`' tracker starts "offline, never synced" until a
    // `ConnectionChanged` says otherwise, and nothing here has dialled
    // anything -- so the pane's very first answer already has to be
    // `Offline`, not the "downloading" wording issue #117 is about.
    assert!(
        settle_until(|| window.reader().absent() == Some(Absent::Offline)),
        "an account that has never connected must not promise a backfill \
         that cannot run: got {:?}",
        window.reader().absent()
    );

    // ── connecting flips it to the ordinary backfill wait ─────────────────
    wired.feeds.apply(&Event::ConnectionChanged {
        account: report.account.id,
        state: ConnectionState::Online,
    });
    assert!(
        settle_until(|| window.reader().absent() == Some(Absent::Partial)),
        "coming online should have turned the offline plate into the \
         ordinary downloading one, without the cursor moving: got {:?}",
        window.reader().absent()
    );

    // ── and losing it again repaints the pane, still with no cursor move ──
    wired.feeds.apply(&Event::ConnectionChanged {
        account: report.account.id,
        state: ConnectionState::Offline,
    });
    assert!(
        settle_until(|| window.reader().absent() == Some(Absent::Offline)),
        "losing the connection again must repaint the pane that is already \
         open, not leave the stale \"downloading\" words on screen: got {:?}",
        window.reader().absent()
    );
}
