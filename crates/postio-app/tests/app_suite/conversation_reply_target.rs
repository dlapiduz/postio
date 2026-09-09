//! US3, and the mistake the whole arrangement exists to prevent: answering
//! the wrong message of a conversation (#1394).
//!
//! The conversation pane has two kinds of verb on screen at once. The bar's
//! `Reply` is thread-level and aims at the **latest** message; the verbs drawn
//! on a message aim at **that** message. ADR 0015 Q4 made them the only
//! per-message verbs in an otherwise thread-level pane, and the whole point is
//! that they can disagree.
//!
//! # What this covers, and what it does not
//!
//! The **app-side** half: that a per-message verb reaches the composer
//! carrying that message, and a thread-level one reaches it carrying the
//! latest. `gtk_conversation.rs` proves the pane raises the right message and
//! stops there — it installs its own handler and never builds a composer.
//! Nothing between the pane and a `Draft` was asserted at all, which is
//! `postio-bl2`: two layers that each pass and are not joined up.
//!
//! It does **not** cover the scope-to-id mapping the one-document pane's
//! in-document links go through. `test_click_reply` calls the pane's handlers
//! directly, so a scope that parsed wrong would not fail this;
//! `gtk_reader.rs` is where the links are proven to raise the right scope.
//! Written down because the gap is the kind that looks covered.
//!
//! # What it asserts, and why `in_reply_to`
//!
//! Not "a composer opened". A composer opening is exactly what the
//! wrong-message failure also looks like — `reply_source.rs` makes the same
//! argument for the same reason. `Draft::in_reply_to` names the message that
//! was actually answered.
//!
//! Three messages, so the thread-level answer and the per-message answer are
//! different ids: aiming a per-message verb at the *newest* would pass whether
//! or not the mapping worked.
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

/// A key press into the main window, through the keymap the application runs.
fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

/// A message in `mailbox`, joined to `thread`.
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

pub fn the_conversations_verbs_answer_the_message_they_name() {
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
    let middle = threaded_message(&database, account.id, inbox, thread, 1, "a reply");
    let newest = threaded_message(&database, account.id, inbox, thread, 2, "the last word");

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
    list.first_row();
    assert!(
        settle_until(|| window.conversation().len() == 3),
        "landing on the thread row never filled the pane with all three messages"
    );

    let composer = window.composer();

    // ── the conversation's own verb answers the latest ───────────────────
    // Not the focused one. The pane opens focused on the newest (FR-015), so
    // this is deliberately checked *again* below from an older focus, where
    // the two answers differ.
    press(&window, "e");
    assert!(
        composer.is_open(),
        "`e` on an open conversation answered nothing"
    );
    assert_eq!(
        composer.draft().in_reply_to,
        Some(newest),
        "a thread-level Reply must answer the latest message of the thread"
    );
    composer.discard();
    settle();

    // ── a per-message verb answers the message it was drawn on ───────────
    // The oldest, so the answer differs from the thread-level one above:
    // aiming this at the newest would pass whether or not the scope-to-id
    // mapping worked at all.
    window.conversation().test_click_reply(oldest);
    settle();
    assert!(composer.is_open(), "a per-message Reply opened nothing");
    assert_eq!(
        composer.draft().in_reply_to,
        Some(oldest),
        "a per-message Reply must answer the message it was drawn on, not the \
         latest -- which is the mistake ADR 0015 Q4's arrangement exists to \
         prevent"
    );
    composer.discard();
    settle();

    // ── and the bar is unmoved by where focus went ───────────────────────
    // FR-008 is about the *bar*, and this is the assertion that makes it mean
    // something: with focus on the middle message the bar must still answer
    // the latest, or "thread-level" is just a second per-message verb wearing
    // the conversation's clothes. The two checks above cannot tell the
    // difference, because the pane opens focused on the newest anyway.
    window.conversation().focus_message(middle);
    settle();
    window
        .conversation()
        .header()
        .actions()
        .press(postio_core::CommandId::Reply);
    settle();
    assert_eq!(
        composer.draft().in_reply_to,
        Some(newest),
        "the conversation bar's Reply followed the focus instead of the thread"
    );
    composer.discard();
    settle();

    // The *keyboard* is a different question, and FR-008 does not answer it:
    // it constrains the bar. `e` here answers the focused message, which for
    // a keyboard-first application is defensible -- you moved to it, you
    // reply to it -- and is asserted so that changing it is a decision rather
    // than a drift. The designer's table wanted `e` for the latest and `⇧e`
    // for the focused one; the maintainer set that table aside, and `⇧e`
    // collides with reply-all besides.
    window.conversation().focus_message(middle);
    settle();
    press(&window, "e");
    assert_eq!(
        composer.draft().in_reply_to,
        Some(middle),
        "`e` inside a conversation answers the message focus is on"
    );
    composer.discard();
    settle();

    window.close();
}
