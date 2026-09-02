//! Issue #436, cause (1): moving the keyboard inside a drilled-in thread
//! never fed the reading pane.
//!
//! #325 fixed exactly this for the main list -- the pane follows the
//! *cursor*, not `connect_activated` (Enter or a double click) --
//! (`cursor_preview.rs` is that test). `ThreadView::connect_activated`
//! already fires on cursor movement too (`select_index` calls `announce`
//! unconditionally), so the widget itself was never the bug. Nothing in the
//! composition root ever subscribed to it: `crates/postio-app/src/
//! reading.rs`, the file that owns what the reading pane shows, had no
//! reference to `window.thread()` at all. So the signal fired into nothing,
//! every test of `thread.rs` in isolation passed, and the column looked
//! broken the moment a person pressed `j` inside it.
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
use postio_model::ids::ThreadId;
use postio_model::{EmailAddress, Message, Thread};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::{Database, test_support};

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

fn press(window: &Window, key: gdk::Key) {
    window.handle_key(key, gdk::ModifierType::empty());
    settle();
}

/// A message in `mailbox`, joined to `thread` -- through
/// `ThreadRepository::add_message`, which recomputes the thread's own
/// aggregates, rather than setting `thread_id` by hand and leaving
/// `message_count` to disagree with what is actually in it.
fn threaded_message(
    database: &Database,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    thread: ThreadId,
    minute: i64,
    subject: &str,
) {
    let connection = database.connection().expect("a connection");
    let mut message = Message::new(
        account,
        mailbox,
        chrono::Utc::now() + chrono::Duration::minutes(minute),
    );
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
    message.subject = Some(subject.to_owned());
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create the threaded message");
    ThreadRepository::new(&connection)
        .add_message(thread, id)
        .expect("join the message to the thread");
}

pub fn moving_the_thread_cursor_fills_the_reading_pane() {
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
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        test_support::account_with_inbox(&connection)
    };
    let thread = {
        let connection = database.connection().expect("a connection");
        let mut thread = Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .expect("create the thread")
    };
    threaded_message(&database, account.id, inbox, thread, 0, "the first message");
    threaded_message(
        &database,
        account.id,
        inbox,
        thread,
        1,
        "the second message",
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let list = window.list();
    // One row, not two: a folder shows one row per conversation (ADR 0015),
    // and the fixture's two messages are one conversation. What this test is
    // about is downstream of that — the thread column's cursor — and the row
    // still has to know it stands for two messages, which the next assertion
    // checks.
    assert!(
        settle_until(|| list.model().n_items() >= 1),
        "the fixture's conversation never reached the list"
    );

    // ── drill in ─────────────────────────────────────────────────────────
    list.first_row();
    let cursor = list.cursor_row().expect("a row to drill into");
    assert_eq!(
        cursor.thread_count, 2,
        "the fixture is wrong if the row does not know its thread has two"
    );
    window.open_thread(&cursor);
    assert!(
        settle_until(|| window.thread().rows().len() == 2),
        "the thread column never filled with both messages"
    );

    let first_in_thread = window
        .thread()
        .cursor()
        .expect("the drill-in leaves the cursor on a message");
    // The conversation pane, not the single-message reader: ADR 0015 Q4 gave
    // the reading pane to the whole conversation and made this column an
    // index into it. What this test is about — that moving the column moves
    // what the pane shows — is unchanged; where to ask is not.
    assert!(
        settle_until(|| window.conversation().len() == 2),
        "opening the thread never filled the reading pane with the \
         conversation"
    );
    assert_eq!(
        window.conversation().focused(),
        Some(first_in_thread),
        "the column and the conversation are one current message, and \
         opening has to leave them agreeing"
    );

    // ── the thread's own cursor move, and the pane follows it ────────────
    // On the bug this is exactly #325's cause B, one surface over: the
    // thread's `k`/`j` move `ThreadView`'s cursor, and nothing told the
    // reading pane.
    press(&window, gdk::Key::k);
    assert!(
        settle_until(|| window.thread().cursor() != Some(first_in_thread)),
        "`k` did not move the thread's own cursor, so this test cannot say \
         anything about what the pane did"
    );
    let second_in_thread = window
        .thread()
        .cursor()
        .expect("the cursor is still on a row");
    assert_ne!(
        first_in_thread, second_in_thread,
        "the thread cursor should be on the other message"
    );

    assert!(
        settle_until(|| window.conversation().focused() == Some(second_in_thread)),
        "moving the cursor inside the thread never moved the conversation \
         pane's focus: it is on {:?}",
        window.conversation().focused()
    );
    assert!(
        window.conversation().is_expanded(second_in_thread),
        "jumping to a message from the index has to open it, or the index \
         cannot reach a message that has already been read"
    );

    bridge.shutdown();
}
