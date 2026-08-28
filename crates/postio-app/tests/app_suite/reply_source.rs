//! Issue #325: `e` on a message the reading pane is showing did nothing.
//!
//! The composer asks a provider what to reply to, and the composition root
//! answered from a `Cell` fed by `List::connect_activated` alone — Enter or a
//! double click. Nobody reads mail that way here: the reading pane follows
//! the *cursor* (#70, Cause B), so `j` over a mailbox left that cell `None`
//! for an entire session and `e`, `E` and `f` were all inert. No composer, no
//! toast, no log line.
//!
//! Every layer underneath passed. `gtk_composer_reply.rs` installs a provider
//! that always answers, so it proves `reply_draft` and the composer; it
//! cannot see that nothing real feeds them. That is `postio-bl2` again, and
//! why this assertion belongs here — at the far end of the key press, over
//! the wiring `feed_the_window` actually builds.
//!
//! # What it asserts, and why `in_reply_to`
//!
//! Not "a composer opened" — that would pass on a reply to the wrong
//! message, which is the failure a `Cell` with a second, differently-updated
//! copy of "the current message" would actually produce. `Draft::in_reply_to`
//! carries the source message's own id, so the assertion names the row the
//! cursor is on. The quoted body carries a per-message marker for the same
//! reason.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket, and this never calls it. The bodies here are written
//! straight into the blob store, exactly as a settled backfill would leave
//! them.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::BodyState;
use postio_model::DraftKind;
use postio_model::ids::MessageId;
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, Database, test_support};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// A key press into the main window, through the keymap the application runs.
fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// The one sentence only this message's body contains.
fn marker(id: MessageId) -> String {
    format!("the body of message {}", id.get())
}

/// Land a body for `id` in the store, the way a settled backfill leaves one.
fn give_body(database: &Database, id: MessageId, text: Option<&str>, html: Option<&str>) {
    let connection = database.connection().expect("a connection");
    let stored = StoredBody {
        text: text.map(str::to_owned),
        html: html.map(str::to_owned),
        headers: None,
    };
    MessageRepository::new(&connection)
        .set_body(id, &stored, BodyState::Full)
        .expect("store the body");
}

/// Move the cursor one row down and answer which message it is on now.
fn next_message(window: &Window) -> MessageId {
    let list = window.list();
    let before = list.cursor_id();
    press(window, "j", gdk::ModifierType::empty());
    assert!(
        settle_until(|| list.cursor_id() != before),
        "`j` did not move the cursor, so nothing below can mean anything"
    );
    list.cursor_id().expect("the cursor is on a row")
}

pub fn reply_forward_and_reply_all_act_on_the_message_under_the_cursor() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-reply-source-{}", std::process::id()));
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
    let report = seed_small(&database, 23);
    assert!(report.message_count > 4, "not enough mail to walk through");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    // ── the same call `run` makes ────────────────────────────────────────
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 4),
        "the list is empty, so there is no cursor to move"
    );
    let composer = window.composer();

    // ── `e` on a window nobody has touched replies to what it is showing ─
    // This used to assert the opposite: the pane was empty until the user
    // moved, so `e` had nothing to answer and did nothing. Since #601 the
    // pane fills for the autoselected row, and the rule underneath is
    // unchanged — `e` replies to the message the reader is *showing* — so
    // the same rule now has the opposite outcome. Worth pinning in that
    // form, because it is what a person meets one keystroke into a fresh
    // window.
    let autoselected = list.cursor_id().expect("a row is autoselected");
    assert!(
        settle_until(|| window.reading()),
        "the window opened without filling the pane"
    );
    press(&window, "e", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`e` answered nothing, though the pane was showing a message"
    );
    assert_eq!(
        composer.draft().in_reply_to,
        Some(autoselected),
        "`e` replied to something other than the message on screen"
    );
    composer.discard();
    settle();

    // ── Return still opens a reply, on a window whose cursor never moved ─
    // Activation was the *only* path in before #325, and it must not
    // regress. Asserted here, before any `j`, so it is genuinely activation
    // being tested: `connect_cursor_moved` has not fired once at this point,
    // and deleting the activation wiring would fail this and nothing else.
    let activated = list.cursor_id().expect("a row is autoselected");
    give_body(
        &database,
        activated,
        Some(&format!("{}\n", marker(activated))),
        None,
    );
    // Through `GtkListView`'s own `list.activate-item`, the way
    // `resume_draft.rs` does it: a key put through `Window::handle_key`
    // never reaches the widget, and the keyboard is not in the list in a
    // test that has been driving it by hand.
    list.test_activate_cursor();
    assert!(
        settle_until(|| window.reading()),
        "activation did not open the message, so nothing below is about \
         replying to it"
    );
    press(&window, "e", gdk::ModifierType::empty());
    assert!(composer.is_open(), "`e` after Return opened nothing");
    assert_eq!(
        composer.draft().in_reply_to,
        Some(activated),
        "activation used to be the only path in, and it must still answer"
    );
    composer.discard();
    settle();

    // ── `e` on the message the cursor is on ──────────────────────────────
    let replied_to = next_message(&window);
    give_body(
        &database,
        replied_to,
        Some(&format!("{}\n", marker(replied_to))),
        None,
    );
    assert!(
        settle_until(|| window.reading()),
        "the pane never filled, so this cannot be a test about what it shows"
    );

    press(&window, "e", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`e` on the message in the reading pane did nothing. The composer's \
         reply source is fed by activation — Enter or a double click — and \
         the pane is fed by the cursor, so replying while reading normally \
         is inert (#325)."
    );
    let draft = composer.draft();
    assert_eq!(draft.kind, DraftKind::Reply);
    assert_eq!(
        draft.in_reply_to,
        Some(replied_to),
        "the composer replied to a different message than the one on screen"
    );
    assert!(
        draft
            .body
            .text
            .as_deref()
            .unwrap_or_default()
            .contains(&marker(replied_to)),
        "the reply quotes nothing of the message it is a reply to"
    );
    // Back to nothing composed, without the discard dialog a person would
    // answer: the gesture under test is the key press, not the teardown.
    composer.discard();
    settle();
    assert!(!composer.is_open());

    // ── `E` on an HTML-only message: markup in, quoted text out ──────────
    // Marketing mail, invitations and anything written in a webmail client
    // are HTML-only, so this is the common case rather than an edge one.
    let replied_all_to = next_message(&window);
    give_body(
        &database,
        replied_all_to,
        None,
        Some(&format!("<p>{}</p>", marker(replied_all_to))),
    );
    press(&window, "E", gdk::ModifierType::SHIFT_MASK);
    assert!(composer.is_open(), "`E` did not open a reply-all (#325)");
    let draft = composer.draft();
    assert_eq!(draft.kind, DraftKind::ReplyAll);
    assert_eq!(draft.in_reply_to, Some(replied_all_to));
    assert!(
        draft
            .body
            .text
            .as_deref()
            .unwrap_or_default()
            .contains(&marker(replied_all_to)),
        "an HTML-only message was quoted as an attribution line with nothing \
         under it"
    );
    composer.discard();
    settle();

    // ── `f` on the message under the cursor ──────────────────────────────
    // A forward carries no `in_reply_to` by design, so the quoted marker is
    // what names the source here.
    let forwarded = next_message(&window);
    give_body(
        &database,
        forwarded,
        Some(&format!("{}\n", marker(forwarded))),
        None,
    );
    press(&window, "f", gdk::ModifierType::empty());
    assert!(composer.is_open(), "`f` did not open a forward (#325)");
    let draft = composer.draft();
    assert_eq!(draft.kind, DraftKind::Forward);
    assert!(
        draft
            .body
            .text
            .as_deref()
            .unwrap_or_default()
            .contains(&marker(forwarded)),
        "the forward carries none of the message it forwards"
    );
    composer.discard();
    settle();

    bridge.shutdown();
}
