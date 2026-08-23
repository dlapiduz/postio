//! The sync engine, against a mock server.
//!
//! Nothing here touches the network: the backend is
//! `postio_imap::backend::MockBackend`, and no SMTP transport is given, so
//! nothing is ever dialled.

use std::sync::Arc;

use chrono::Utc;
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_imap::backend::{Fault, MockBackend};
use postio_model::MailboxRole;
use postio_model::operation::{Operation, OperationTarget};
use postio_runtime::engine::{Engine, EngineParts, Link, NetworkState};
use postio_storage::repository::OperationQueueRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// An engine over a seeded database and a mock server, with an event stream to
/// read what it announced.
fn engine() -> (
    Engine,
    postio_storage::Database,
    postio_storage::seed::SeedReport,
    EventStream,
) {
    let (engine, database, report, events, _backend) = engine_with_backend();
    (engine, database, report, events)
}

/// As [`engine`], keeping the mock so a test can make it fail.
fn engine_with_backend() -> (
    Engine,
    postio_storage::Database,
    postio_storage::seed::SeedReport,
    EventStream,
    Arc<MockBackend>,
) {
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, events) = event_channel();

    let backend = Arc::new(MockBackend::new());
    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        // Never dialled: nothing in these tests queues a send, and the
        // connector is only consulted when one does.
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
    })
    .expect("the engine starts");

    (engine, database, report, events, backend)
}

#[tokio::test]
async fn an_empty_queue_drains_to_nothing() {
    let (engine, _database, _report, _events) = engine();

    let summary = engine
        .drain()
        .await
        .expect("draining an empty queue is not a failure");

    assert!(
        summary.is_empty(),
        "a queue with nothing in it did something: {summary:?}"
    );
}

#[tokio::test]
async fn a_drain_settles_the_rows_it_finds() {
    // The whole point of postio-avl: the queue filled up locally and nothing
    // ever carried it anywhere, so every row sat pending for ever.
    //
    // What is asserted is the plumbing — a session is opened, the queue is
    // read, and no row is left pending. Whether IMAP accepted the flag is
    // `postio-sync`'s to test and it does; the mock here holds no matching
    // message, so this row settles as obsolete rather than applied, which is
    // still the drain doing its job.
    let (engine, database, report, _events) = engine();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let message = queue_a_flag_change(&database, &report, inbox.id);
    let _ = message;

    let summary = engine.drain().await.expect("a drain pass");

    assert!(
        !summary.is_empty(),
        "the queued row was never looked at: {summary:?}"
    );
    let connection = database.connection().expect("a connection");
    let still_pending = OperationQueueRepository::new(&connection)
        .pending(report.account.id, Utc::now())
        .expect("the queue reads");
    assert!(
        still_pending.is_empty(),
        "the row was left pending after a drain: {still_pending:?}"
    );
}

#[tokio::test]
async fn seeding_the_backfill_finds_bodies_worth_having() {
    // postio-26c: `seed` existed and nothing called it, so no body was ever
    // fetched for a message the user had not opened.
    let (engine, _database, report, _events) = engine();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let queued = engine
        .seed_backfill(inbox.id, 50)
        .await
        .expect("seeding reads the store");

    assert!(
        queued <= 50,
        "seeding asked for more than the limit it was given"
    );
}

#[tokio::test]
async fn a_message_nobody_has_is_nothing_to_fetch() {
    let (engine, _database, _report, _events) = engine();

    let wanted = engine
        .request_body(postio_model::ids::MessageId::new(987_654))
        .await
        .expect("asking about a message that is not here is not a failure");

    assert!(!wanted, "there was nothing to fetch and it said so");
}

#[tokio::test]
async fn the_engine_answers_after_the_handle_is_cloned() {
    // Cloning gives another handle to the same thread; both have to work, or
    // the composition root cannot hand one to each surface that needs it.
    let (engine, _database, _report, _events) = engine();
    let second = engine.clone();

    let (first, second) = tokio::join!(engine.drain(), second.drain());
    first.expect("the first handle works");
    second.expect("and so does the clone");
}

#[tokio::test]
async fn a_connection_that_will_not_open_leaves_the_queue_where_it_is() {
    // Local-first: the write already happened here. Reaching the server is a
    // separate thing that can wait, so a drain with no session is a
    // connection problem and never an operation that failed — the row has to
    // still be there when the connection comes back.
    let (engine, database, report, events, backend) = engine_with_backend();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    let message = queue_a_flag_change(&database, &report, inbox.id);
    // Twice: the engine asks `capabilities` first — the cheap question that
    // answers from a session already open — and only dials when that says
    // there is none. Both have to fail for there to be no session at all.
    backend.inject(Fault::AuthFailed);
    backend.inject_after(1, Fault::AuthFailed);

    let error = engine
        .drain()
        .await
        .expect_err("the credentials were refused");
    assert!(!error.message().is_empty());

    let connection = database.connection().expect("a connection");
    let pending = OperationQueueRepository::new(&connection)
        .pending(report.account.id, Utc::now())
        .expect("the queue reads");
    assert_eq!(
        pending.len(),
        1,
        "the queued row was settled by a failure to connect"
    );
    let _ = message;

    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::ConnectionChanged {
                state: postio_core::ConnectionState::Failing,
                ..
            }
        )),
        "the UI was not told the connection is the problem"
    );
}

#[tokio::test]
async fn a_refused_password_blocks_and_a_new_one_unblocks() {
    // A refused password does not get better on a timer, so nothing retries
    // it until someone says the credentials have changed. That is the one
    // thing `retry_now` is for.
    let (engine, _database, _report, events, backend) = engine_with_backend();
    backend.inject(Fault::AuthFailed);
    backend.inject_after(1, Fault::AuthFailed);

    engine
        .drain()
        .await
        .expect_err("the credentials were refused");
    let blocked = engine.link().await.expect("the engine answers");
    assert!(
        matches!(blocked, Link::Blocked(_)),
        "a refused password should stop the link, not back it off: {blocked:?}"
    );
    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::ConnectionChanged {
                state: postio_core::ConnectionState::Failing,
                ..
            }
        )),
        "the status line was not told"
    );

    // A new password, and the link is worth trying again.
    let moved = engine.retry_now().await.expect("the engine answers");
    assert!(
        !matches!(moved, Link::Blocked(_)),
        "a new password should clear the block: {moved:?}"
    );
}

#[tokio::test]
async fn a_connection_that_dies_mid_drain_parks_the_link_at_once() {
    // The point of `observe`. A session that died part-way through has
    // already cost the user one action; waiting for the next poll to admit it
    // costs another. Whatever hit the broken connection tells the supervisor
    // directly.
    let (engine, database, report, _events, backend) = engine_with_backend();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    queue_a_flag_change(&database, &report, inbox.id);

    // Connect, so the link is genuinely up first.
    engine.drain().await.expect("a first pass connects");
    assert!(
        engine.link().await.expect("the engine answers").is_online(),
        "the link should be up before this test means anything"
    );

    queue_a_flag_change(&database, &report, inbox.id);
    backend.inject(Fault::Disconnect);
    let error = engine.drain().await.expect_err("the session died");
    assert!(!error.message().is_empty());

    let link = engine.link().await.expect("the engine answers");
    assert!(
        matches!(link, Link::Waiting { .. }),
        "the link should be backing off already, without a poll: {link:?}"
    );
}

#[tokio::test]
async fn no_network_is_not_a_backoff() {
    // `Link::Offline` is deliberately not `Link::Waiting`: with no network
    // there is nothing to retry against, so attempts are not spent and the
    // status line says "offline" rather than counting down to a reconnection
    // that cannot succeed.
    let (engine, _database, _report, events, _backend) = engine_with_backend();

    let link = engine
        .set_network(NetworkState::Down)
        .await
        .expect("the engine answers");
    assert_eq!(link, Link::Offline);
    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::ConnectionChanged {
                state: postio_core::ConnectionState::Offline,
                ..
            }
        )),
        "the status line was not told the machine is offline"
    );

    // And a drain while offline leaves the queue alone rather than failing it.
    let error = engine
        .drain()
        .await
        .expect_err("there is nothing to send over");
    assert!(
        error.message().contains("network"),
        "the reason should name the network: {}",
        error.message()
    );
}

/// Queue one flag change against the newest message in `mailbox`.
fn queue_a_flag_change(
    database: &postio_storage::Database,
    report: &postio_storage::seed::SeedReport,
    mailbox: postio_model::ids::MailboxId,
) -> postio_model::ids::MessageId {
    let connection = database.connection().expect("a connection");
    let page = postio_storage::repository::MessageRepository::new(&connection)
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(mailbox),
            limit: 1,
            after: None,
        })
        .expect("the inbox has mail");
    let message = page.first().expect("at least one message").id;
    OperationQueueRepository::new(&connection)
        .enqueue(
            report.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: postio_model::FlagSet::from_iter([postio_model::Flag::Seen]),
            },
            Utc::now(),
        )
        .expect("the row queues");
    message
}

/// What the engine announced, drained without blocking.
fn announced(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}
