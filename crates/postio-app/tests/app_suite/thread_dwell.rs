//! Issue #754: reading a conversation from the main window marks its
//! messages read — per message, driven by focus, per ADR 0015 Q4.
//!
//! Before #755 the main window never *displayed* a thread's older unread
//! members, so nothing could mark them: the list dwell fired one
//! `MarkReadOnDwell` for the representative, the thread's `unread_count`
//! stayed above zero, and the row stayed bold for ever. Now that landing on
//! a thread row opens the conversation pane, each message's own dwell fires
//! as focus reaches it — and this proves the whole path: focus → dwell →
//! command → SQLite → queue → the *row* un-bolding without a folder switch.
//!
//! That last assertion is the one that failed longest (#754, consequence
//! 3): `pages_holding` resolves a `MessagesChanged` announcement against
//! row ids, the conversation row's id is its representative, so a
//! non-representative member marked read refetched no page until the
//! announcement learnt to name the whole conversation.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wired, Wiring, commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{MessageId, ThreadId};
use postio_model::{EmailAddress, Flag, Message, Thread};
use postio_storage::repository::{MessageRepository, OperationQueueRepository, ThreadRepository};
use postio_storage::{Database, test_support};

/// Short enough that the test does not spend a real second per message,
/// long enough to stay distinguishable from "marked on focus".
const DWELL: Duration = Duration::from_millis(80);

/// Whether the store says `message` carries `\Seen`.
fn is_read(database: &Database, message: MessageId) -> bool {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .expect("a read")
        .expect("the message is still there")
        .flags
        .contains(&Flag::Seen)
}

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

pub fn resting_inside_a_conversation_reads_each_message_as_focus_reaches_it() {
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
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        test_support::account_with_inbox(&connection)
    };
    // Two conversations, because the gesture under test is a cursor
    // *move*. The list draws newest first, so the recent one is row 0 and
    // gets the autoselect — which must read nothing (#71/#601) — and `j`
    // lands on the older one, which is a person choosing a row.
    let new_thread = || {
        let connection = database.connection().expect("a connection");
        let mut thread = Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .expect("create the thread")
    };
    let members_of = |thread: ThreadId, from_minute: i64| -> Vec<MessageId> {
        (0..3)
            .map(|n| {
                threaded_message(
                    &database,
                    account.id,
                    inbox,
                    thread,
                    from_minute + n,
                    &format!("interlock, message {n}"),
                )
            })
            .collect()
    };
    let untouched = members_of(new_thread(), 100);
    let members = members_of(new_thread(), 0);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // A *real* bus, exactly as `dwell_wiring.rs` argues: the question is
    // whether focus reaches a verb that writes to SQLite.
    let state = SharedState::default();
    let bus = postio_app::actions::wire(
        Dispatcher::builder(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
    // `run`'s own arrangement, because the last assertion is about a
    // *repaint*: one hub the bus emits into, and one window subscription
    // drained into the panes. A test that skipped the drain would prove the
    // write and nothing about the row (`window_drain.rs` is why this shape
    // exists).
    let hub = EventHub::new();
    let engine = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(bus, hub.sink())
        .expect("a runtime");
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        engine,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let Wired { feeds, .. } =
        feed_the_window(&window, &wiring).expect("the seeded store has an account");
    commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        bus_verbs,
    );
    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    commands::drain(&window, &feeds, hub.subscribe("window"), notifier);
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() == 2),
        "the fixture's two conversations never reached the list"
    );
    window.conversation().set_dwell_delay(DWELL);

    // ── the window opened on a conversation, and read none of it ─────────
    // The autoselect shows the newest conversation, which is right (#601),
    // and must not start anybody's read-clock (#71) — the unread signal
    // destroying itself is precisely what that rule prevents.
    assert!(
        settle_until(|| window.conversation().len() == 3),
        "the autoselected row never opened its conversation"
    );
    std::thread::sleep(DWELL * 4);
    while glib::MainContext::default().iteration(false) {}
    assert!(
        untouched.iter().all(|id| !is_read(&database, *id)),
        "launching Postio read the conversation it happened to open on"
    );

    // ── `j` onto the other conversation: a row a person chose ────────────
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
    assert!(
        settle_until(|| window.conversation().rows().first().map(|row| row.id) == Some(members[0])),
        "`j` did not open the other conversation, so nothing below is about it"
    );
    assert_eq!(
        window.conversation().focused(),
        Some(members[0]),
        "the conversation opens focused on the first unread"
    );

    // ── each message is read as focus rests on it, and only then ─────────
    assert!(
        settle_until(|| is_read(&database, members[0])),
        "resting on the first unread never marked it read"
    );
    assert!(
        !is_read(&database, members[1]) && !is_read(&database, members[2]),
        "opening a conversation must not read messages focus never reached — \
         'opened the thread, all six read' is exactly what ADR 0015 Q4 forbids"
    );

    // Newest first, then back to the middle — so the *last* message read is
    // not the one whose id the list row carries. The row stands for its
    // representative, which is the newest member (`feed::thread_row`), and a
    // walk that happened to end there would flip the row on that message's
    // own announcement and prove nothing about #754's repaint gap. Ending on
    // a non-representative member is what makes the last assertion depend on
    // the announcement naming the whole conversation.
    for member in [members[2], members[1]] {
        window.conversation().focus_message(member);
        assert!(
            settle_until(|| is_read(&database, member)),
            "focus rested on a message and it never went read"
        );
    }

    // ── the server hears about each, exactly once ────────────────────────
    let queued = {
        let connection = database.connection().expect("a connection");
        OperationQueueRepository::new(&connection)
            .pending(account.id, chrono::Utc::now())
            .expect("a read")
    };
    assert_eq!(
        queued.len(),
        3,
        "one \\Seen per message read, and none for the conversation nobody \
         opened: {queued:?}"
    );

    // ── and the row stops being bold, with no folder switch ──────────────
    // The row's id is the representative's; the members marked read are not
    // it. This is #754's consequence 3: the repaint announcement has to
    // name the conversation, or `pages_holding` finds no page to refetch.
    assert!(
        settle_until(|| list.cursor_row().is_some_and(|row| row.seen)),
        "every member is read and the conversation row still draws unread; \
         the page holding it was never refetched"
    );

    bridge.shutdown();
}
