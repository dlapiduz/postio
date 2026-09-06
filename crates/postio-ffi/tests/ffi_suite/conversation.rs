//! A conversation crosses the boundary whole (#1263, ADR 0015 Q4).
//!
//! The macOS pane stacks every message of a thread. Two things have to be
//! true for that to be possible at all: the boundary can name a conversation
//! (`ScopeFfi::Thread`), and what comes back is the *whole* thread rather than
//! the part of it that happens to be filed in the folder on screen. The
//! second is what these hold — a thread routinely spans Inbox and Archive,
//! and a pane that showed only the visible half would be lying quietly.
//!
//! The fold itself is `postio_ui::conversation`'s, tested there. What is
//! tested here is that the boundary applies it rather than leaving it to
//! whichever frontend asked.

use chrono::{DateTime, TimeZone, Utc};
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{ConversationFfi, ScopeFfi, Session, SessionOptions};
use postio_model::{AccountId, EmailAddress, Flag, MailboxId, Message, Thread};
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// One message of the conversation, in `mailbox`, read or not.
fn message(
    connection: &postio_storage::PooledConnection,
    account: AccountId,
    mailbox: MailboxId,
    sender: &str,
    seconds: i64,
    seen: bool,
) -> Message {
    let mut message = Message::new(account, mailbox, at(seconds));
    message.subject = Some("Radon reduction".to_owned());
    message.from = vec![EmailAddress::new(
        Some(sender),
        format!("{}@example.com", sender.to_lowercase()),
    )];
    message.preview = Some(format!("Snippet {seconds}"));
    message.flags = if seen {
        [Flag::Seen].into_iter().collect()
    } else {
        Default::default()
    };
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("a message");
    message
}

/// A store holding one conversation, and a session over it.
///
/// Four messages: three in the inbox and one filed in Archive, so "every
/// message in the thread" is a claim with something to prove. The two oldest
/// have been read.
fn a_conversation() -> (std::sync::Arc<Session>, i64, Vec<i64>) {
    let database = test_support::memory();
    let (thread, ids, inbox) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let archive = test_support::mailbox(&connection, &account, "Archive");

        let mut thread = Thread::new(account.id);
        thread.subject = Some("radon reduction".to_owned());
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).expect("a thread");

        // Out of order on purpose: what arrives from the store is not what
        // the pane stacks, and the ordering is the boundary's to apply.
        let third = message(&connection, account.id, inbox, "Ada", 300, false);
        let first = message(&connection, account.id, archive.id, "Ada", 100, true);
        let fourth = message(&connection, account.id, inbox, "Quinn", 400, false);
        let second = message(&connection, account.id, inbox, "Quinn", 200, true);
        for message in [&first, &second, &third, &fourth] {
            threads.add_message(thread.id, message.id).expect("add");
        }
        (
            thread.id.get(),
            vec![
                first.id.get(),
                second.id.get(),
                third.id.get(),
                fourth.id.get(),
            ],
            inbox,
        )
    };

    let state = SharedState::default();
    let bus = postio_session::actions::wire(
        Dispatcher::builder(),
        postio_session::actions::Actions::new(database.clone(), state),
    )
    .build();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let bridge = Box::leak(Box::new(bridge));
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone())
            .on_bridge(bridge.handle(), bridge.commands()),
    )
    .expect("a session over the store");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: inbox.into(),
    });
    (session, thread, ids)
}

/// Ask for the conversation and wait for the read the way a frontend would
/// not have to: it repaints when the event arrives.
fn read(session: &Session, thread: i64) -> ConversationFfi {
    session.open_conversation(thread);
    session.settle_for_test();
    session.conversation().expect("the conversation was read")
}

#[test]
fn a_conversation_arrives_whole_and_oldest_first() {
    let (session, thread, ids) = a_conversation();
    let conversation = read(&session, thread);

    assert_eq!(
        conversation
            .rows
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        ids,
        "every message of the thread, stacked the way it was had — including \
         the one filed in Archive rather than the folder on screen"
    );
    assert_eq!(conversation.thread, thread);
}

#[test]
fn the_boundary_folds_the_conversation_rather_than_the_frontend() {
    let (session, thread, _) = a_conversation();
    let conversation = read(&session, thread);

    // Two read, then the first unread: that is where a pane opens, and it is
    // `postio_ui::conversation`'s rule rather than Swift's.
    assert_eq!(conversation.focus, Some(2));
    assert_eq!(
        conversation.expanded,
        vec![false, false, true, true],
        "the read ones stay one line; the unread ones open"
    );
}

#[test]
fn the_header_says_how_many_messages_and_who_is_in_it() {
    let (session, thread, _) = a_conversation();
    let conversation = read(&session, thread);

    assert_eq!(conversation.subject, "Radon reduction");
    assert!(
        conversation.meta.starts_with("4 messages · Ada, Quinn · "),
        "the header names the count and the people: {}",
        conversation.meta
    );
    assert!(
        conversation.meta.split(" · ").count() == 3,
        "count, people, span: {}",
        conversation.meta
    );
}

#[test]
fn a_thread_nobody_has_opened_has_no_conversation() {
    let (session, _, _) = a_conversation();
    assert!(
        session.conversation().is_none(),
        "the pane draws nothing until a conversation has been asked for"
    );
}

#[test]
fn a_thread_that_is_not_there_reads_as_empty_rather_than_stale() {
    let (session, thread, _) = a_conversation();
    let _ = read(&session, thread);

    session.open_conversation(404);
    session.settle_for_test();
    let conversation = session.conversation().expect("a conversation, empty");
    assert_eq!(conversation.thread, 404);
    assert!(
        conversation.rows.is_empty(),
        "a pane must not keep drawing the last conversation under a new one"
    );
    assert_eq!(conversation.focus, None);
}

// -- the folded run (canvas turn 8a) -----------------------------------------

/// A row from `sender`, for a run's summary line.
fn row(id: i64, sender: &str) -> postio_ffi::RowFfi {
    postio_ffi::RowFfi {
        id,
        thread: Some(1),
        is_thread: false,
        from: Some(sender.to_owned()),
        from_address: Some(format!("{}@example.com", sender.to_lowercase())),
        initials: sender.chars().take(1).collect(),
        subject: Some("Radon reduction".to_owned()),
        preview: None,
        received_at: 1_770_000_000 + id,
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
    }
}

#[test]
fn three_collapsed_messages_in_a_row_become_one_divider() {
    let rows = vec![
        row(1, "Ada"),
        row(2, "Bo"),
        row(3, "Ada"),
        row(4, "Quinn"),
        row(5, "Ada"),
    ];
    let runs = postio_ffi::conversation_runs(rows, vec![true, false, false, false, true]);

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].start, 1);
    assert_eq!(runs[0].count, 3);
    assert_eq!(
        runs[0].summary, "3 earlier messages · Bo, Ada, Quinn",
        "a divider names how many it hides and who is in it"
    );
}

#[test]
fn two_collapsed_messages_are_left_as_two_lines() {
    // A divider hides its messages behind a click, so it has to save more
    // lines than it costs. Two become one plus a gesture, which is no saving.
    let rows = vec![row(1, "Ada"), row(2, "Bo"), row(3, "Ada")];
    let runs = postio_ffi::conversation_runs(rows, vec![true, false, false]);
    assert!(runs.is_empty());
}

#[test]
fn a_conversation_with_nothing_folded_has_no_dividers() {
    let rows = vec![row(1, "Ada"), row(2, "Bo")];
    assert!(postio_ffi::conversation_runs(rows, vec![true, true]).is_empty());
}
