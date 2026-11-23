use chrono::{DateTime, TimeZone, Utc};
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{ConversationFfi, Session, SessionOptions};
use postio_model::{AccountId, EmailAddress, Flag, MailboxId, Message, Thread};
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

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

async fn a_conversation() -> (std::sync::Arc<Session>, i64, Vec<i64>) {
    let database = test_support::memory().await;
    let (thread, ids, _inbox) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let archive = test_support::mailbox(&connection, &account, "Archive").await;

        let mut thread = Thread::new(account.id);
        thread.subject = Some("radon reduction".to_owned());
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");

        // Out of order on purpose: the ordering is the boundary's to apply.
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
    (session, thread, ids)
}

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

    // The newest message, not the first unread (as in GTK); the rule is
    // `postio_ui::conversation`'s.
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
