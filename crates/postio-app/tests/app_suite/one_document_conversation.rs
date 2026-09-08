//! ADR 0032's conversation, in the app, on a real store (#1316).
//!
//! `gtk_reader` proves one view costs one web process whatever the thread's
//! length, and `postio-ui`'s `reader::thread` proves the document is composed
//! correctly. Neither can fail if nothing ever calls them, which is #327 and
//! `postio-bl2`: a capability implemented, tested, documented and wired to
//! nothing, green throughout.
//!
//! So this starts where the application starts — a seeded thread, a real
//! `Window`, `feed_the_window`, and the same `open_conversation` a click
//! makes — and asserts on the document that actually reached WebKit.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound.

use crate::{settle, settle_until};
use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::{BlobStore, Database, test_support};

/// A message in `thread`, `minutes` after the epoch of this test, with a body.
fn threaded_message(
    database: &Database,
    account: AccountId,
    mailbox: MailboxId,
    thread: ThreadId,
    minutes: i64,
    subject: &str,
    body: &str,
) -> MessageId {
    let connection = database.connection().expect("a connection");
    let mut message = postio_model::Message::new(
        account,
        mailbox,
        chrono::Utc::now() + chrono::Duration::minutes(minutes),
    );
    message.subject = Some(subject.to_string());
    message.from = vec![postio_model::EmailAddress::new(
        Some("Ada Lovelace"),
        "ada@example.com",
    )];
    message.sync.body_state = postio_model::BodyState::Full;
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create the message");
    // Through the repository, which is what actually joins a message to a
    // thread -- setting `thread_id` on the struct leaves the thread with no
    // members and the conversation opens holding one row.
    ThreadRepository::new(&connection)
        .add_message(thread, id)
        .expect("join the message to the thread");
    MessageRepository::new(&connection)
        .set_body(
            id,
            &StoredBody {
                text: Some(body.to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .expect("store the body");
    id
}

/// The four bodies, so the wait and the assertions cannot drift apart.
const BODIES: [&str; 4] = [
    "the first one",
    "the second one",
    "the third one",
    "the last one",
];

pub fn a_thread_opens_as_one_document_holding_every_message() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test, before the app runs.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", state_dir.path());
        std::env::set_var("POSTIO_ONE_DOCUMENT", "1");
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        test_support::account_with_inbox(&connection)
    };
    let thread = {
        let connection = database.connection().expect("a connection");
        let mut thread = postio_model::Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .expect("create the thread")
    };
    for (index, body) in BODIES.iter().enumerate() {
        threaded_message(
            &database,
            account.id,
            inbox,
            thread,
            index as i64,
            &format!("message {index}"),
            body,
        );
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();
    let wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the seeded thread never reached the list"
    );
    assert!(
        window.conversation().is_one_document(),
        "POSTIO_ONE_DOCUMENT was set and the pane is still stacked, so nothing \
         below is testing what it says"
    );

    list.first_row();
    let cursor = list.cursor_row().expect("a row to land on");
    window.open_conversation(&cursor);

    // Every message, in one document. Waiting for four, because the bodies
    // arrive one at a time and the pane redraws once they have settled.
    // Wait for the *bodies*, not for the row count: four `<details>` is
    // satisfied by four rows that are still empty, which is exactly the
    // half-drawn state this is meant to catch.
    let filled = settle_until(|| {
        window
            .conversation()
            .thread_document()
            .is_some_and(|document| {
                document.matches("<details").count() == 4
                    && BODIES.iter().all(|body| document.contains(body))
            })
    });
    let document = window
        .conversation()
        .thread_document()
        .expect("the pane composed a document");
    assert!(
        filled,
        "the thread opened but the document holds {} of 4 messages. Every \
         layer under this one passes; that is the shape of bug #327 is about \
         -- check what is between them.",
        document.matches("<details").count()
    );

    for body in BODIES {
        assert!(
            document.contains(body),
            "the document is missing {body:?}, so a message drew without its body"
        );
    }
    assert_eq!(
        document.matches("<!DOCTYPE html>").count(),
        1,
        "four messages should be one document, not four"
    );

    let _ = wired;
    bridge.shutdown();
}
