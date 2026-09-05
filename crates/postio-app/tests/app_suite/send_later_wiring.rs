//! Can a person actually schedule a message to send later in the running
//! application?
//!
//! `crates/postio-gtk/tests/gtk_composer_schedule_send.rs` proves
//! `ctrl+shift+Return` opens the picker. That is not the same claim as this
//! one: #423 showed that a composer signal with no caller anywhere in the
//! workspace reports success right up until someone asks the database what
//! happened, so this starts where the application starts — a seeded store, a
//! real `Window`, and `feed_the_window`, the call `run` makes — calls
//! [`postio_gtk::composer::Composer::send_later`] the way a chosen popover
//! row would, and then asks the *store*, not the widget.
//!
//! Nothing here touches the network: `start_syncing` is never called, so the
//! `Operation::Send` this leaves in the queue simply waits there — first for
//! its `next_attempt_at`, same as it would after a restart, and only then
//! for a transport that is never started in this test either.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use chrono::{Duration, Utc};
use gtk::gdk;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{DraftState, Operation};
use postio_session::Wiring;
use postio_storage::repository::{DraftRepository, OperationQueueRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const SUBJECT: &str = "Tide gate interlock, follow-up";
const RECIPIENT: &str = "quinn@example.net";

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

pub fn choosing_a_time_schedules_the_draft_for_sending() {
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
    let report = seed_small(&database, 29);
    let account = report.account.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
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
    settle();

    // ── the same call `run` makes: this is what wires the composer ───────
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    settle();

    let composer = window.composer();
    press(&window, "c", gdk::ModifierType::empty());
    assert!(composer.is_open(), "`c` did not reach the composer");
    composer.test_set_to(RECIPIENT);
    composer.test_set_subject(SUBJECT);
    composer.test_set_body("Nothing to add; scheduling the recap for the morning.");
    settle();

    // ── ctrl+shift+Return reaches the picker, exactly as the GTK-level ────
    // test already proves; what this test is about is the *choice*, so it
    // calls the same method a chosen row's action would.
    press(
        &window,
        "Return",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    // Truncated to millisecond precision: that is what the queue's own
    // schema stores a timestamp as, and the row this leaves is read back
    // through that same truncation.
    let send_at = Utc::now() + Duration::hours(3);
    let send_at =
        chrono::DateTime::from_timestamp_millis(send_at.timestamp_millis()).expect("in range");
    composer.send_later(send_at);
    settle();

    // ── and now ask the store, not the widget ────────────────────────────
    let connection = database.connection().expect("a connection");
    let queue = OperationQueueRepository::new(&connection);
    let all_pending = queue
        .pending(account, send_at + Duration::minutes(1))
        .expect("read the queue");
    let sent = all_pending
        .iter()
        .find_map(|row| match row.operation {
            Operation::Send { draft } => Some((draft, row.next_attempt_at)),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "choosing a time left no Operation::Send in the queue — the \
                 composer's send-later seam reaches nothing. Status line \
                 says: {:?}",
                composer.status()
            )
        });

    assert_eq!(
        sent.1,
        Some(send_at),
        "the operation must carry the chosen time, not the moment it was queued"
    );

    // ── and must not drain before that time, restart or not ──────────────
    let too_early = queue
        .pending(account, send_at - Duration::minutes(1))
        .expect("read the queue");
    assert!(
        too_early
            .iter()
            .all(|row| row.operation != Operation::Send { draft: sent.0 }),
        "the scheduled send must not be offered to the drainer before its time"
    );

    let draft = DraftRepository::new(&connection)
        .get(sent.0)
        .expect("read the draft")
        .expect("the queued send names a draft that is not in the store");
    assert_eq!(draft.state, DraftState::Queued);
    assert_eq!(draft.subject, SUBJECT);

    assert!(
        !composer.is_open(),
        "scheduling a send closes the composer, the same way sending does"
    );

    bridge.shutdown();
}
