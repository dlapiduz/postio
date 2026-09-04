//! Issue #755: landing on a thread row opens the conversation, no `t`
//! required.
//!
//! ADR 0015 Q4: "The column is an index. The pane is the conversation."
//! Opening a thread row — the cursor landing on it, a click, `Enter` —
//! shows the whole conversation, focused on the first unread, expanded and
//! scrolled to. There is no second gesture and no second surface (#1003).
//!
//! On the bug, `Fill::fill` read `row.id` and never asked `row.is_thread()`,
//! so every one of those gestures showed a single message — the newest in
//! the folder — and the conversation existed only behind `t`.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use crate::settle_until;
use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{MessageId, ThreadId};
use postio_model::{EmailAddress, Message, Thread};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::{Database, test_support};

/// A message in `mailbox`, joined to `thread` — through
/// `ThreadRepository::add_message` so the aggregates agree with the rows.
fn threaded_message(
    database: &Database,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    thread: ThreadId,
    minute: i64,
    subject: &str,
) -> MessageId {
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
    id
}

pub fn landing_on_a_thread_row_opens_the_conversation() {
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
    let oldest = threaded_message(
        &database,
        account.id,
        inbox,
        thread,
        0,
        "the opening message",
    );
    let newest = threaded_message(&database, account.id, inbox, thread, 1, "the reply");

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = postio_storage::BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

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
    assert!(
        settle_until(|| list.model().n_items() >= 1),
        "the fixture's conversation never reached the list"
    );

    // ── the cursor lands on the thread row, and that is the whole gesture ─
    // There is no second gesture to make: landing here opens the
    // conversation, and since #1003 there is no drill-in key at all.
    list.first_row();
    let cursor = list.cursor_row().expect("a row to land on");
    assert!(
        cursor.is_thread(),
        "the fixture is wrong if the folder row does not stand for the \
         conversation"
    );

    assert!(
        settle_until(|| window.conversation().len() == 2),
        "landing on the thread row never filled the reading pane with the \
         whole conversation; it holds {} message(s)",
        window.conversation().len()
    );
    assert!(
        window.conversation().widget().is_visible(),
        "the conversation pane filled but is not the surface on screen"
    );
    // Focus opens on the first unread — the oldest here, both being unread —
    // expanded, per the pane's own opening policy.
    assert_eq!(
        window.conversation().focused(),
        Some(oldest),
        "the conversation must open focused on the first unread message"
    );
    assert!(
        window.conversation().is_expanded(oldest),
        "the focused message must open expanded, or focus points at a \
         closed door"
    );

    // ── `Enter` on the same row is the same answer, not a downgrade ──────
    window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
    settle();
    assert_eq!(
        window.conversation().len(),
        2,
        "activating the row must keep the conversation, not swap in a \
         single message"
    );
    assert!(
        window.conversation().widget().is_visible(),
        "activating the row hid the conversation pane"
    );
    let _ = newest;

    bridge.shutdown();
}
