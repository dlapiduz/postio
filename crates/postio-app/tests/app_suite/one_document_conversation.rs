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
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::{BlobStore, Database, test_support};

/// A message in `thread`, `minutes` after the epoch of this test, with a body.
/// Where a seeded message goes: the account, mailbox and thread it joins.
struct Seat {
    account: AccountId,
    mailbox: MailboxId,
    thread: ThreadId,
}

fn threaded_message(
    database: &Database,
    seat: &Seat,
    minutes: i64,
    subject: &str,
    body: &str,
    seen: bool,
) -> MessageId {
    let (account, mailbox, thread) = (seat.account, seat.mailbox, seat.thread);
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
    // Who it went to, which the document has to say (#1427). Two, so the
    // drawn line is the plain list rather than the "and N others" form --
    // the shortening has its own tests in `postio_ui::reader::header`.
    message.to = vec![
        postio_model::EmailAddress::new(Some("Grace Hopper"), "grace@example.com"),
        postio_model::EmailAddress::new(None::<&str>, "bob@example.com"),
    ];
    message.sync.body_state = postio_model::BodyState::Full;
    if seen {
        message.flags.insert(postio_model::Flag::Seen);
    }
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

/// The one message seeded unread, and so drawn open.
const UNREAD: usize = 3;

/// How many messages the thread under test holds.
///
/// Twelve rather than four, because the claim being tested is about *scale*:
/// bodies arriving must not cost a document each. At four, "coalesced" and
/// "did not" are two apart and the assertion turns on timing; at twelve the
/// difference is unmistakable however loaded the machine.
const MESSAGES: usize = 12;

/// The body of message `index`, so the wait and the assertions cannot drift.
fn body_of(index: usize) -> String {
    format!("the body of message {index}")
}

/// Which messages the document currently draws open, by scope.
fn open_messages(window: &Window) -> Vec<String> {
    let document = window.conversation().thread_document().unwrap_or_default();
    let mut open = Vec::new();
    for piece in document
        .split("<details class=\"postio-message\" id=\"m-")
        .skip(1)
    {
        let Some((scope, rest)) = piece.split_once('"') else {
            continue;
        };
        if rest.starts_with(" open>") {
            open.push(scope.to_string());
        }
    }
    open
}

/// The one-document pane is what a conversation opens as, with nobody asking
/// for it (#1316, ADR 0032 Accepted).
///
/// It shipped behind `POSTIO_ONE_DOCUMENT` while ADR 0032 was Proposed, and
/// the variable's own comment said why: *"an experiment with a decision still
/// to be made, and `config.toml` is where settled choices live."* The
/// decision is made, so the experiment is the default and the variable is
/// gone.
///
/// Deliberately sets **nothing**. Every other case in this file turns the
/// pane on through `OneDocument::on()` and would pass against a build that
/// still needed asking; this is the one that would not.
/// An open message says who it went to (#1427).
///
/// The stacked pane drew `To` and `Cc` on every expanded message through
/// `postio_ui::reader::header::recipient_line`. The one-document pane drew
/// the sender and stopped -- so #1316, making it the default, made the
/// application say less about a message than it had.
///
/// The count is the information: "and 197 others" is what stops a reply-all,
/// and #1332 kept the full list in a tooltip for the same reason. Asserted on
/// the document that actually reached WebKit.
pub fn an_open_message_says_who_it_went_to() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test, before the app runs.
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
    let seat = Seat {
        account: account.id,
        mailbox: inbox,
        thread,
    };
    for index in 0..2 {
        threaded_message(
            &database,
            &seat,
            index as i64,
            &format!("message {index}"),
            &body_of(index),
            true,
        );
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the seeded thread never reached the list"
    );
    list.first_row();
    let cursor = list.cursor_row().expect("a row to land on");
    window.open_conversation(&cursor);
    assert!(
        settle_until(|| window.conversation().len() == 2),
        "opening the thread never filled the pane"
    );

    // The control: the document exists and holds both messages, so a failure
    // below is about recipients rather than about nothing having rendered.
    assert!(
        settle_until(|| window
            .conversation()
            .thread_document()
            .is_some_and(|document| document.matches("<details").count() == 2)),
        "the document never drew both messages"
    );

    assert!(
        settle_until(|| window
            .conversation()
            .thread_document()
            .is_some_and(|document| document.contains("grace@example.com")
                && document.contains("bob@example.com"))),
        "the open message does not say who it went to. The stacked pane drew \
         To and Cc on every expanded message; one document drew the sender \
         and stopped (#1427)"
    );

    window.destroy();
}

pub fn a_conversation_opens_as_one_document_without_being_asked() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test, before the app runs.
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
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // An account and one thread: `feed_the_window` needs an account to feed
    // from, and the pane is only asked what shape it is once there is
    // something for it to be that shape about.
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
    let seat = Seat {
        account: account.id,
        mailbox: inbox,
        thread,
    };
    for index in 0..2 {
        threaded_message(
            &database,
            &seat,
            index as i64,
            &format!("message {index}"),
            &body_of(index),
            true,
        );
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");

    assert!(
        window.conversation().is_one_document(),
        "a conversation pane still opens stacked with nothing asking it to. \
         ADR 0032 is Accepted and `POSTIO_ONE_DOCUMENT` is gone; the pane is \
         supposed to need no persuading"
    );

    window.destroy();
}

pub fn a_thread_opens_as_one_document_holding_every_message() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test, before the app runs.
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
    // All read but one in the middle, so "open" means something specific:
    // the unread one, and the newest. `UNREAD` is the message this test then
    // marks read, which is what resting on it does.
    let seat = Seat {
        account: account.id,
        mailbox: inbox,
        thread,
    };
    let mut ids = Vec::new();
    for index in 0..MESSAGES {
        ids.push(threaded_message(
            &database,
            &seat,
            index as i64,
            &format!("message {index}"),
            &body_of(index),
            index != UNREAD,
        ));
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let database_handle = database.clone();
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
        "the pane is stacked, so nothing below is testing what it says. It \
         is the default since ADR 0032 was accepted (#1316), so this failing \
         means the default moved rather than that a variable went unset"
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
                document.matches("<details").count() == MESSAGES
                    && (0..MESSAGES).all(|index| document.contains(&body_of(index)))
            })
    });
    let document = window
        .conversation()
        .thread_document()
        .expect("the pane composed a document");
    assert!(
        filled,
        "the thread opened but the document holds {} of {MESSAGES} messages. \
         Every layer under this one passes; that is the shape of bug #327 is \
         about -- check what is between them.",
        document.matches("<details").count()
    );

    for index in 0..MESSAGES {
        let body = body_of(index);
        assert!(
            document.contains(&body),
            "the document is missing {body:?}, so a message drew without its body"
        );
    }
    assert_eq!(
        document.matches("<!DOCTYPE html>").count(),
        1,
        "{MESSAGES} messages should be one document, not {MESSAGES}"
    );

    // Every load is a full teardown and reload -- JavaScript is off, so there
    // is no incremental path -- and the bodies of a thread arrive one at a
    // time. Rendering on arrival cost one load per message, which is the
    // "first time is slower, going back is instant" the maintainer noticed:
    // going back costs nothing because `<details>` toggles in a DOM that is
    // already parsed, while arriving cost the whole document again.
    let renders = window.conversation().thread_renders();
    assert!(
        renders <= 4,
        "a {MESSAGES}-message thread cost {renders} documents. Bodies arriving \
         one at a time have to coalesce, or a thread costs a full teardown and \
         reload per message on the way to showing it -- which is what made the \
         first open of a thread slower than every return to it"
    );

    // ── reopening the thread must not move what is open ────────────────
    //
    // A redraw recomputes the document, and everything about which messages
    // are open used to be recomputed with it -- from `seen` and from where
    // focus is. Both move on their own: resting on a message marks it read,
    // which made it collapse under the reader; and the pane reopens whenever
    // the thread is re-read. Expansion is the reader's state, not a function
    // of the model, and a reload cannot be allowed to take it.
    let open_before = open_messages(&window);
    assert!(
        !open_before.is_empty(),
        "nothing was open after the thread filled, so this proves nothing"
    );
    let renders_before = window.conversation().thread_renders();

    // Resting on a message marks it read. The store says so, the thread is
    // re-read, and the pane redraws -- and the message the reader is looking
    // at must not fold shut underneath them because of it.
    {
        let connection = database_handle.connection().expect("a connection");
        let mut flags = postio_model::FlagSet::default();
        flags.insert(postio_model::Flag::Seen);
        MessageRepository::new(&connection)
            .set_flags(
                ids[UNREAD],
                &flags,
                postio_storage::repository::FlagSource::Local,
            )
            .expect("mark it read");
    }

    window.open_conversation(&cursor);
    assert!(
        settle_until(|| window
            .conversation()
            .thread_document()
            .is_some_and(|document| {
                document.matches("<details").count() == MESSAGES
                    && (0..MESSAGES).all(|index| document.contains(&body_of(index)))
            })),
        "reopening the thread never refilled it"
    );

    assert_eq!(
        open_messages(&window),
        open_before,
        "reopening the thread changed which messages are open"
    );
    let refill = window.conversation().thread_renders() - renders_before;
    assert!(
        refill <= 2,
        "reopening the same thread cost {refill} documents; the bodies were \
         already in hand and nothing about the thread changed"
    );

    let _ = wired;
    bridge.shutdown();
}

/// A conversation of one message still offers its verbs (#1349).
///
/// The footer stands down for a single message, and the comment on that
/// condition says exactly why: *"drawing both bars put `Reply` on screen twice
/// with `e` on each (#1173); the lone message carries `Archive` in its own bar
/// instead"*.
///
/// All true for the stacked pane. In the one-document pane the per-message
/// chrome is HTML in the document, so there is no second bar to collide with
/// and nothing left to carry the verbs — the footer stands down and the pane
/// offers no way to reply with the mouse at all. That is #1259's complaint,
/// which is the reason `postio_ui::reader::header` exists.
///
/// Rendered side by side to find it: the stacked pane draws four buttons, the
/// one-document pane draws none.
pub fn a_single_message_conversation_still_offers_its_verbs() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: single-threaded test, before the app runs.
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
    let seat = Seat {
        account: account.id,
        mailbox: inbox,
        thread,
    };
    threaded_message(&database, &seat, 0, "message 0", &body_of(0), true);

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    settle();
    // Held to the end of the test: dropping the wiring tears down the feeds
    // that keep the pane filled.
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the seeded message never reached the list"
    );
    assert!(
        window.conversation().is_one_document(),
        "the pane is stacked, so nothing below is testing what it says. It \
         is the default since ADR 0032 was accepted (#1316), so this failing \
         means the default moved rather than that a variable went unset"
    );

    list.first_row();
    let cursor = list.cursor_row().expect("a row to land on");
    window.open_conversation(&cursor);
    assert!(
        settle_until(|| window.conversation().thread_document().is_some()),
        "the single-message conversation never composed a document"
    );

    // Whichever bar is on screen: what matters is that a person can reply,
    // not which branch built the widget.
    let footer = window
        .conversation()
        .visible_actions()
        .expect("no action bar is on screen at all");
    assert!(
        footer.is_visible(),
        "a conversation of one message offers no verbs at all: the footer \
         stood down for a per-message bar that does not exist in this pane, so \
         there is no way to reply with the mouse. That is #1259."
    );
    // `ArchiveThread`, not `Archive`: spec FR-008 scopes reply, reply all and
    // forward to the latest message and archive to the **whole conversation**,
    // which is what a person means by archiving a thread. The first draft of
    // this test asked for `Archive` and was wrong about the requirement rather
    // than finding a bug.
    for command in [
        CommandId::Reply,
        CommandId::ReplyAll,
        CommandId::Forward,
        CommandId::ArchiveThread,
    ] {
        assert!(
            footer.button(command).is_some(),
            "{command:?} is not on the bar"
        );
    }
}
