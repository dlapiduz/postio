//! Issue #739: a conversation entry does not repaint when its body arrives.
//!
//! `body_arrives.rs` proved `Event::BodyLoaded` reaches the single reading
//! pane (#396). The conversation pane (ADR 0015 Q4, #308) is a second,
//! independent pane that can be showing the same message and was not wired
//! to the same event at all: an expanded entry whose body was still
//! downloading kept its "Downloading this message" plate after the bytes
//! landed, exactly as the single pane did before #396.
//!
//! Driven from the real composition root (`feed_the_window`) with a real
//! store and `Feeds::apply` handed the event a real engine would emit, for
//! the same reason `body_arrives.rs` is: the bug was in the wiring between
//! layers, not in `ConversationView` or `Fill` in isolation.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::Event;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{BodyState, Flag, Message};
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
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

/// The mirror of [`settle_until`], for a criterion about something *not*
/// happening: a repaint that should never have been queued cannot be waited
/// for, only ruled out.
fn settle_while(held: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        settle();
        if !held() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    held()
}

/// One message, headers-only, joined to `thread`.
fn threaded_message(
    database: &Database,
    account: AccountId,
    mailbox: MailboxId,
    thread: ThreadId,
    minute: i64,
    subject: &str,
    seen: bool,
) -> MessageId {
    let connection = database.connection().expect("a connection");
    let mut message = Message::new(
        account,
        mailbox,
        chrono::Utc::now() + chrono::Duration::minutes(minute),
    );
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::HeadersOnly;
    if seen {
        message.flags.insert(Flag::Seen);
    }
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create the threaded message");
    ThreadRepository::new(&connection)
        .add_message(thread, id)
        .expect("join the message to the thread");
    id
}

/// Write `text` as `message`'s body, as a completed fetch leaves it.
fn store_body(database: &Database, message: MessageId, text: &str) {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .set_body(
            message,
            &StoredBody {
                text: Some(text.to_owned()),
                html: None,
                headers: None,
            },
            BodyState::Full,
        )
        .expect("the body is stored");
}

pub fn a_body_that_lands_repaints_the_conversation_entry_waiting_for_it_and_no_other() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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

    // Two read (collapsed on open), then two unread (expanded, well under
    // the cap) -- `target` is the one this test repaints, `sibling` is the
    // other expanded entry, kept absent throughout to prove an arrival for
    // `target` does not leak onto it.
    let collapsed = threaded_message(&database, account.id, inbox, thread, 0, "first", true);
    threaded_message(&database, account.id, inbox, thread, 1, "second", true);
    let target = threaded_message(&database, account.id, inbox, thread, 2, "third", false);
    let sibling = threaded_message(&database, account.id, inbox, thread, 3, "fourth", false);

    // A message outside this conversation entirely -- its own thread, never
    // opened here.
    let other_thread = {
        let connection = database.connection().expect("a connection");
        let mut thread = postio_model::Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .expect("create the other thread")
    };
    // Older than every message of the conversation under test, so it is
    // never the newest row and `list.first_row()` still lands on that
    // conversation.
    let foreign = threaded_message(
        &database,
        account.id,
        inbox,
        other_thread,
        -10,
        "unrelated",
        false,
    );

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
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

    let wired = feed_the_window(&window, &wiring).expect("the store has an account");
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the seeded conversation never reached the list"
    );

    list.first_row();
    let cursor = list.cursor_row().expect("a row to drill into");
    window.open_thread(&cursor);
    assert!(
        settle_until(|| window.conversation().len() == 4),
        "opening the thread never filled the conversation pane"
    );
    assert!(
        settle_until(|| window.conversation().is_expanded(target)
            && window.conversation().is_expanded(sibling)),
        "both unread messages should have opened expanded"
    );
    assert!(
        !window.conversation().is_expanded(collapsed),
        "a read message should not have opened expanded"
    );

    let reader = |message: MessageId| {
        window
            .conversation()
            .reader_for(message)
            .expect("an expanded entry has a reader")
    };
    // Which wait it explains -- online vs offline (#117) -- is not this
    // issue's concern; only that it is waiting on a body it has not got.
    assert!(
        settle_until(|| reader(target).absent().is_some()),
        "the entry should be waiting on a body it has not got: got {:?}",
        reader(target).absent()
    );

    // ── 1. an arrival for a message not showing in this entry ───────────
    //
    // `target`'s own body is written *first*, so an indiscriminate repaint
    // would visibly flip its entry to a rendered body here. Only a consumer
    // that reads the event's `message` and asks the conversation pane which
    // entry it belongs to can leave the plate up for the wrong arrivals.
    let waiting = reader(target).absent();
    store_body(&database, target, "the third message landed");
    wired.feeds.apply(&Event::BodyLoaded {
        account: account.id,
        message: collapsed,
    });
    wired.feeds.apply(&Event::BodyLoaded {
        account: account.id,
        message: foreign,
    });
    assert!(
        settle_while(|| reader(target).absent() == waiting),
        "a body arriving for a collapsed entry, or for a message outside the \
         conversation, repainted an unrelated entry anyway"
    );
    assert_eq!(
        reader(sibling).absent(),
        waiting,
        "the sibling entry, which never got a body, must stay on its wait"
    );

    // ── 2. and one for the message the entry is expanded on ─────────────
    let before = reader(target).paints();
    wired.feeds.apply(&Event::BodyLoaded {
        account: account.id,
        message: target,
    });
    assert!(
        settle_until(|| reader(target).absent().is_none()),
        "the body for the expanded entry landed and it went on showing the \
         wait -- check that anything at all consumes `BodyLoaded` for the \
         conversation pane"
    );
    assert_eq!(
        reader(target).paints() - before,
        1,
        "one arrival should be one repaint"
    );

    // ── 3. and it did so once, not once per event in a burst ────────────
    let once = reader(target).paints();
    for _ in 0..20 {
        wired.feeds.apply(&Event::BodyLoaded {
            account: account.id,
            message: target,
        });
    }
    assert!(
        settle_until(|| reader(target).paints() > once),
        "the coalesced repaint for the burst never happened"
    );
    assert_eq!(
        reader(target).paints() - once,
        1,
        "twenty arrivals in one burst should be one repaint, not {}",
        reader(target).paints() - once
    );

    bridge.shutdown();
}
