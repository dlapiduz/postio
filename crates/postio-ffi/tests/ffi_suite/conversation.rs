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
async fn message(
    connection: &postio_storage::Checkout,
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
        .await
        .expect("a message");
    message
}

/// A store holding one conversation, and a session over it.
///
/// Four messages: three in the inbox and one filed in Archive, so "every
/// message in the thread" is a claim with something to prove. The two oldest
/// have been read.
async fn a_conversation() -> (std::sync::Arc<Session>, i64, Vec<i64>) {
    let database = test_support::memory().await;
    let (thread, ids, inbox) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let archive = test_support::mailbox(&connection, &account, "Archive").await;

        let mut thread = Thread::new(account.id);
        thread.subject = Some("radon reduction".to_owned());
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");

        // Out of order on purpose: what arrives from the store is not what
        // the pane stacks, and the ordering is the boundary's to apply.
        let third = message(&connection, account.id, inbox, "Ada", 300, false).await;
        let first = message(&connection, account.id, archive.id, "Ada", 100, true).await;
        let fourth = message(&connection, account.id, inbox, "Quinn", 400, false).await;
        let second = message(&connection, account.id, inbox, "Quinn", 200, true).await;
        for message in [&first, &second, &third, &fourth] {
            threads
                .add_message(thread.id, message.id)
                .await
                .expect("add");
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

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_arrives_whole_and_oldest_first() {
    let (session, thread, ids) = a_conversation().await;
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

#[tokio::test(flavor = "multi_thread")]
async fn the_boundary_folds_the_conversation_rather_than_the_frontend() {
    let (session, thread, _) = a_conversation().await;
    let conversation = read(&session, thread);

    // The most recent message: that is where a pane opens, and it is
    // `postio_ui::conversation`'s rule rather than Swift's.
    //
    // It used to be the first unread, which is index 2 here. FR-015 moved it
    // to the newest — a thread is opened to read the latest thing said in it,
    // and "first unread" put the pane somewhere in the middle of a thread
    // somebody had already half-read. GTK made that move first; this is the
    // assertion that the macOS pane came with it rather than keeping the old
    // rule in the shared crate.
    assert_eq!(conversation.focus, Some(3));
    assert_eq!(
        conversation.expanded,
        vec![false, false, true, true],
        "the focused one opens, and expansion walks back from it over the \
         unread; the two already read stay one line"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_header_says_how_many_messages_and_who_is_in_it() {
    let (session, thread, _) = a_conversation().await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_nobody_has_opened_has_no_conversation() {
    let (session, _, _) = a_conversation().await;
    assert!(
        session.conversation().is_none(),
        "the pane draws nothing until a conversation has been asked for"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_that_is_not_there_reads_as_empty_rather_than_stale() {
    let (session, thread, _) = a_conversation().await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_message_can_be_asked_for_as_a_row_without_a_list_position() {
    // The single-message pane's whole problem. `rowAt` answers by *index*
    // into whatever list is open, and a message the store has not threaded
    // is drawn from an id with no list under it -- so the pane had a
    // message id, no way to turn it into a row, and therefore no sender, no
    // subject, no date and no actions.
    let (session, _thread, ids) = a_conversation().await;
    let message = *ids.first().expect("a message");

    let row = session.row_for(message).expect("a row for the message");
    assert_eq!(row.id, message);
    assert!(
        row.from.is_some(),
        "the row carries no sender, so a header built from it would say nothing"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_that_is_gone_is_no_row_rather_than_an_empty_one() {
    // A row full of blanks reads as a message with no sender, which is a
    // statement about somebody's mail. Nothing is the truthful answer.
    let (session, _thread, _ids) = a_conversation().await;
    assert!(session.row_for(9_999).is_none());
    session.shutdown();
}
