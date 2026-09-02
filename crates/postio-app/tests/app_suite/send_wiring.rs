//! Can a person actually send a message in the running application?
//!
//! `crates/postio-gtk/tests/gtk_composer_*.rs` prove the composer hands its
//! draft to whatever is connected to `connect_send`. That is not the same
//! claim, and #423 is the gap between them: **nothing ever connected**.
//! `Composer::connect_send` had no caller anywhere in the workspace from the
//! composer's first commit, so `Composer::send` found its handler list empty
//! every time and set the status line to "no outgoing account is connected
//! yet" — wording plausible enough to read as a configuration problem rather
//! than as a seam that was never wired. No message had ever been sendable
//! through the UI, on any account, in any state.
//!
//! The composer's own tests could not see it, for the same reason the ones in
//! #325 could not: they install a handler themselves, so the seam they are
//! asserting about is one they created. So this starts where the application
//! starts — a seeded store, a real `Window`, and `feed_the_window`, the call
//! `run` makes — presses `ctrl+Return`, and then asks the *database* what
//! happened. Nothing here reaches into the composer to trigger a send.
//!
//! Nothing here touches the network: `start_syncing` is never called, so the
//! `Operation::Send` this leaves in the queue simply waits there, which is
//! precisely the local-first promise being asserted — the send is durable
//! before any SMTP transport is involved at all.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
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

const SUBJECT: &str = "Tide gate interlock";
const RECIPIENT: &str = "quinn@example.net";

/// A key press into the main window. GTK4 gives no supported way to
/// synthesize a GDK event, so this is the same call the window's own
/// controller makes — one line below where a real key would land.
fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

pub fn ctrl_return_queues_the_draft_for_sending() {
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
    let report = seed_small(&database, 13);
    let account = report.account.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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
    assert!(!composer.is_open(), "nothing is being composed yet");

    // ── write a message, through the keys and fields a person uses ───────
    press(&window, "c", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`c` did not reach the composer, so this test cannot say anything \
         about sending"
    );
    composer.test_set_to(RECIPIENT);
    composer.test_set_subject(SUBJECT);
    composer.test_set_body("The interlock trips at half tide.");
    settle();

    // ── ctrl+Return, travelling the registry, keymap and dispatch ────────
    press(&window, "Return", gdk::ModifierType::CONTROL_MASK);

    // ── and now ask the store, not the widget ────────────────────────────
    let connection = database.connection().expect("a connection");
    let queued = OperationQueueRepository::new(&connection)
        .pending(account, chrono::Utc::now())
        .expect("read the queue");
    let sent = queued
        .iter()
        .find_map(|row| match row.operation {
            Operation::Send { draft } => Some(draft),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "ctrl+Return left no Operation::Send in the queue — the \
                 composer's send seam reaches nothing. Status line says: {:?}. \
                 Every layer under this one passes; that is the shape of bug \
                 #423 is about.",
                composer.status()
            )
        });

    let draft = DraftRepository::new(&connection)
        .get(sent)
        .expect("read the draft")
        .expect(
            "the queued send names a draft that is not in the store — the \
             close path deleted the row the operation has to build its bytes \
             from, so the send would drain as obsolete",
        );
    assert_eq!(
        draft.state,
        DraftState::Queued,
        "a draft handed to the queue is Queued: the composer has let go of \
         it, and it is the drainer's now until `postio-sync::send` deletes \
         the row and files the Sent copy"
    );
    assert_eq!(
        draft.subject, SUBJECT,
        "and it is the message that was typed"
    );
    assert_eq!(
        draft.to.len(),
        1,
        "with the recipient it was addressed to still on it"
    );
    assert_eq!(draft.to[0].address, RECIPIENT);

    // ── the UI did not wait for SMTP, and did not lie about why ──────────
    assert!(
        !composer.is_open(),
        "sending closes the composer; the message is the queue's problem now"
    );
    assert!(
        !composer.status().contains("no outgoing account"),
        "the status line still claims there is nowhere to send to, on an \
         account that is configured: {:?}",
        composer.status()
    );

    bridge.shutdown();
}
