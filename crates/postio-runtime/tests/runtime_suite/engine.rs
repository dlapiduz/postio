use std::sync::Arc;

use chrono::Utc;
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_model::MailboxRole;
use postio_model::operation::{Operation, OperationTarget};
use postio_runtime::engine::{Engine, EngineParts, Link, NetworkSource, NetworkState, SystemClock};
use postio_storage::repository::OperationQueueRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

use crate::harness;

use harness::BlobDir;
use postio_storage::bind;

async fn engine() -> (
    Engine,
    postio_storage::Store,
    postio_storage::seed::SeedReport,
    EventStream,
    BlobDir,
) {
    let (engine, database, report, events, _backend, directory) = engine_with_backend().await;
    (engine, database, report, events, directory)
}

async fn engine_with_backend() -> (
    Engine,
    postio_storage::Store,
    postio_storage::seed::SeedReport,
    EventStream,
    Arc<MockBackend>,
    BlobDir,
) {
    engine_with(|_| {}).await
}

/// `prepare` runs on the mock before `Engine::spawn`: a fault installed after races the
/// supervisor's first connection, and a drain with an empty queue then makes no backend call
/// (#330).
async fn engine_with(
    prepare: impl FnOnce(&MockBackend),
) -> (
    Engine,
    postio_storage::Store,
    postio_storage::seed::SeedReport,
    EventStream,
    Arc<MockBackend>,
    BlobDir,
) {
    engine_with_backfill(prepare, Default::default()).await
}

/// The interactive body path needs the background lane off, or the two race for the same message.
async fn engine_with_backfill(
    prepare: impl FnOnce(&MockBackend),
    backfill: postio_sync::BackfillPolicy,
) -> (
    Engine,
    postio_storage::Store,
    postio_storage::seed::SeedReport,
    EventStream,
    Arc<MockBackend>,
    BlobDir,
) {
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();

    let backend = Arc::new(server());
    prepare(&backend);
    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill,
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    let directory = BlobDir::new(engine.clone(), directory);
    (engine, database, report, events, backend, directory)
}

#[tokio::test]
async fn an_empty_queue_drains_to_nothing() {
    let (engine, _database, _report, _events, _directory) = engine().await;

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
    // Asserts the plumbing: a session opens, the queue is read, no row is left pending. The mock
    // holds no matching message, so the row settles as obsolete.
    let (engine, database, report, _events, _directory) = engine().await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let message = queue_a_flag_change(&database, &report, inbox.id).await;
    let _ = message;

    engine.drain().await.expect("a drain pass");

    // The engine drains on its own when the link comes up, so this explicit drain may find the
    // queue already empty.
    let still_pending = with_store(&database, "reading the queue", async |connection| {
        let connection = &connection;
        OperationQueueRepository::new(connection)
            .pending(report.account.id, Utc::now())
            .await
    })
    .await;
    assert!(
        still_pending.is_empty(),
        "the row was left pending after a drain: {still_pending:?}"
    );
}

#[tokio::test]
async fn seeding_the_backfill_finds_bodies_worth_having() {
    let (engine, _database, report, _events, _directory) = engine().await;
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

/// `set_latency` makes overlap observable without a stopwatch: a serial loop never has two fetches
/// in front of it (#1551).
#[tokio::test(flavor = "multi_thread")]
async fn the_body_backfill_keeps_more_than_one_fetch_in_flight() {
    use std::time::Duration;

    let (engine, database, report, _events, backend, _directory) =
        engine_with(|mock| mock.set_latency(Duration::from_millis(10))).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let candidates = give_the_inbox_uids(&database, inbox.id).await;
    assert!(
        candidates > 0,
        "the fixture found no messages to give uids to"
    );
    engine
        .seed_backfill(inbox.id, 10)
        .await
        .expect("seeding reads the store");

    let fetched = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let progress = engine
                .backfill_progress()
                .await
                .expect("the engine answers");
            if progress.stored >= 3 {
                return progress;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        fetched.is_ok(),
        "fewer than three bodies were fetched, so this proves nothing about \
         how many were in flight"
    );

    assert!(
        backend.peak_in_flight() > 1,
        "the backfill never had two bodies in front of the server, so it is \
         still fetching one at a time and a first sync pays one round trip \
         per message: peak {}",
        backend.peak_in_flight()
    );
    drop(engine);
}

#[tokio::test]
async fn a_seeded_body_is_actually_fetched() {
    let (engine, database, report, _events, _directory) = engine().await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let candidates = give_the_inbox_uids(&database, inbox.id).await;

    let queued = engine
        .seed_backfill(inbox.id, 10)
        .await
        .expect("seeding reads the store");

    // Not `assert!(queued > 0)`: the running loop may already have queued these, so `queued` can be
    // 0 with nothing wrong (#851).
    assert!(
        candidates > 0,
        "the fixture found no messages to give uids to"
    );

    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let progress = engine
                .backfill_progress()
                .await
                .expect("the engine answers");
            if progress.pending == 0 && progress.in_flight == 0 {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the backfill loop never settled — nothing is claiming bodies");

    assert!(
        settled.stored > 0,
        "no body ever actually arrived, so this proves only that the loop \
         settles failures: {settled:?}"
    );
    assert!(
        settled.stored + settled.gone + settled.failed + settled.skipped >= queued,
        "fewer bodies settled than were queued: {settled:?}"
    );
}

#[tokio::test]
async fn a_backfill_says_how_far_it_has_got_without_being_asked() {
    let (engine, database, report, events, _directory) = engine().await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    let candidates = give_the_inbox_uids(&database, inbox.id).await;

    let queued = engine
        .seed_backfill(inbox.id, 10)
        .await
        .expect("seeding reads the store");
    let _ = queued;
    assert!(
        candidates > 0,
        "the fixture found no messages to give uids to"
    );

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let progress = engine
                .backfill_progress()
                .await
                .expect("the engine answers");
            if progress.pending == 0 && progress.in_flight == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the backfill loop never settled");

    let reports: Vec<(u32, u32)> = announced(&events)
        .into_iter()
        .filter_map(|event| match event {
            Event::BackfillProgress { done, total, .. } => Some((done, total)),
            _ => None,
        })
        .collect();

    assert!(
        !reports.is_empty(),
        "the backfill fetched bodies and announced none of it, so the status \
         line has nothing to draw but `idle`"
    );

    let (last_done, last_total) = *reports.last().expect("checked non-empty");
    assert!(
        last_done >= queued as u32,
        "the final report claims {last_done} settled out of {queued} queued"
    );
    assert_eq!(
        last_done, last_total,
        "a drained queue must report done == total, or the sidebar reads \
         `downloading` for as long as the account stays connected"
    );
    // Not `reports[0].0 == 0`: nothing guarantees the seed event precedes the running loop's first
    // settle (#851);
    // the first thing said just must not be "finished".
    let (first_done, first_total) = reports[0];
    assert!(
        first_done < first_total,
        "the first report already claimed the queue was drained, so nothing \
         was said while there was anything to say: {reports:?}"
    );

    assert!(
        reports.iter().any(|(done, total)| done < total),
        "every report claimed the queue was already drained, so the status \
         line would never once read as downloading: {reports:?}"
    );
    assert!(
        reports.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "the settled count went backwards, which is not progress: {reports:?}"
    );
    assert!(
        reports.iter().all(|(done, total)| done <= total),
        "a report claimed more settled than were ever queued: {reports:?}"
    );
}

#[tokio::test]
async fn a_sync_pass_puts_the_servers_mail_in_the_local_store() {
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let (engine, events, _directory) = engine_over(&database, account.id, server());

    let summary = engine.sync(mailbox.id).await.expect("a sync pass");
    assert!(
        summary.inserted > 0,
        "the server had mail and none of it arrived: {summary:?}"
    );

    let stored = stored_in(&database, mailbox.id).await;
    assert_eq!(
        stored, summary.inserted,
        "the pass said it wrote rows the store does not have"
    );

    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::MessageListChanged { mailbox: changed, .. } if *changed == mailbox.id
        )),
        "nothing told the open list its mailbox had changed"
    );

    // A first sync's messages are history, not new mail; `NewMail` here would be a notification
    // storm.
    assert!(
        !announced(&events)
            .iter()
            .any(|event| matches!(event, Event::NewMail { .. })),
        "an initial sync must never announce new mail"
    );
}

#[tokio::test]
async fn a_resync_that_finds_new_mail_announces_it() {
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    let _ = announced(&events);

    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(arriving_message()),
        )
        .await
        .expect("the server takes delivery");
    engine.sync(mailbox.id).await.expect("a resync");

    let arrived = announced(&events)
        .into_iter()
        .find_map(|event| match event {
            Event::NewMail {
                mailbox, messages, ..
            } => Some((mailbox, messages)),
            _ => None,
        })
        .expect("the delivery must be announced as new mail");
    assert_eq!(arrived.0, mailbox.id);
    assert_eq!(arrived.1.len(), 1, "exactly the one message that arrived");
}

fn reply_to_the_first(n: u32) -> Vec<u8> {
    format!(
        "From: Grace Hopper <grace@example.org>\r\n\
         To: Ada Lovelace <ada@example.com>\r\n\
         Subject: Re: body 1\r\n\
         Message-ID: <reply-{n}@example.org>\r\n\
         In-Reply-To: <body-1@example.com>\r\n\
         References: <body-1@example.com>\r\n\
         Date: Mon, 1 Jun 2026 10:0{n}:00 +0000\r\n\
         \r\n\
         Reply {n}.\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn replies_arriving_into_a_thread_are_announced_once_for_the_thread() {
    // Two replies in one pass are one `ThreadChanged` naming both; a first sync names none.
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    assert!(
        !announced(&events)
            .iter()
            .any(|event| matches!(event, Event::ThreadChanged { .. })),
        "a first sync's messages are not arrivals"
    );

    for n in 1..=2 {
        backend
            .append(
                "INBOX",
                &postio_account::backend::AppendMessage::new(reply_to_the_first(n)),
            )
            .await
            .expect("the server takes delivery");
    }
    engine.sync(mailbox.id).await.expect("a resync");

    let ids = |rfc: &str| {
        let database = database.clone();
        let rfc = postio_model::RfcMessageId::new(rfc);
        let account = account.id;
        async move {
            let connection = database.connect().await.expect("a connection");
            postio_storage::repository::MessageRepository::new(&connection)
                .ids_by_rfc_message_id(account, &rfc)
                .await
                .expect("a read")
        }
    };
    let first = ids("<body-1@example.com>").await[0];
    let replies = [
        ids("<reply-1@example.org>").await[0],
        ids("<reply-2@example.org>").await[0],
    ];
    let thread = {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .get(first)
            .await
            .expect("a read")
            .expect("the message")
            .thread_id
            .expect("it is threaded")
    };

    let changed: Vec<_> = announced(&events)
        .into_iter()
        .filter_map(|event| match event {
            Event::ThreadChanged {
                account: to,
                thread,
                mut members_added,
            } => {
                assert_eq!(to, account.id);
                members_added.sort();
                Some((thread, members_added))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        changed,
        vec![(thread, replies.to_vec())],
        "one event for the thread, naming both replies"
    );
}

#[tokio::test]
async fn a_modseq_less_resync_that_finds_new_mail_announces_it() {
    // A server with no MODSEQ re-reads the folder every pass, so its steady-state pass is a full
    // one and its deliveries must still announce (#1704); the first pass announces nothing.
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=3u32 {
        inbox = inbox.message(MockMessage::new(numbered(n)));
    }
    let backend = Arc::new(
        MockBackend::builder()
            .capabilities(["IMAP4rev1"])
            .mailbox(inbox)
            .build(),
    );
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    assert!(
        !announced(&events)
            .iter()
            .any(|event| matches!(event, Event::NewMail { .. })),
        "a first sync must never announce new mail"
    );

    for n in 4..=5u32 {
        backend
            .append(
                "INBOX",
                &postio_account::backend::AppendMessage::new(numbered(n)),
            )
            .await
            .expect("the server takes delivery");
    }
    engine.sync(mailbox.id).await.expect("a resync");

    let arrived: Vec<_> = announced(&events)
        .into_iter()
        .filter_map(|event| match event {
            Event::NewMail { messages, .. } => Some(messages),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(arrived.len(), 2, "exactly the two messages that arrived");
}

fn numbered(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: numbered {n}\r\n\
         Message-ID: <numbered-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
         \r\n\
         Number {n}.\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn mail_arriving_on_a_resync_is_an_arrival_rather_than_a_reload() {
    // A pass that only delivered mail must emit `NewMail` and not also `MessageListChanged`, whose
    // invalidate discards every row widget (#72).
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    let _ = announced(&events);

    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(arriving_message()),
        )
        .await
        .expect("the server takes delivery");
    engine.sync(mailbox.id).await.expect("a resync");

    let seen = announced(&events);
    assert!(
        seen.iter().any(|event| matches!(
            event,
            Event::NewMail { mailbox: to, messages, .. } if *to == mailbox.id && messages.len() == 1
        )),
        "the arrival itself must still be announced: {seen:?}"
    );
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            Event::MessageListChanged { mailbox: changed, .. } if *changed == mailbox.id
        )),
        "a pass that only delivered mail also demanded a full reload, which \
         throws away every visible row widget and re-reads every page while \
         the user is trying to read: {seen:?}"
    );
}

#[tokio::test]
async fn a_finished_sync_queues_the_bodies_it_just_learned_about() {
    // A sync is exactly when the set of messages missing a body changes, so exactly when to seed.
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let (engine, _events, _directory) = engine_over(&database, account.id, server());

    assert_eq!(
        engine
            .backfill_progress()
            .await
            .expect("the engine answers")
            .pending,
        0
    );

    engine.sync(mailbox.id).await.expect("a sync pass");

    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let progress = engine
                .backfill_progress()
                .await
                .expect("the engine answers");
            if progress.pending == 0 && progress.in_flight == 0 && progress.stored > 0 {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the sync never seeded any body worth fetching");

    assert!(
        settled.stored > 0,
        "the sync brought mail in and none of its bodies followed: {settled:?}"
    );
}

/// A long download must not make the app deaf to new mail: the loop drains the body queue before it
/// watches for arrivals (#1551).
#[tokio::test(flavor = "multi_thread")]
async fn mail_that_arrives_during_a_long_download_is_still_noticed() {
    use std::time::Duration;

    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, _events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    let synced = settle(&database, mailbox.id).await;
    assert!(synced > 0);

    // A queue long enough to outlast the slice on a slow server. The rows name uids the mock lacks
    // on purpose: it charges latency on entry, so each costs a round trip.
    {
        let connection = database.connect().await.expect("a connection");
        let mut message = postio_model::Message::new(account.id, mailbox.id, chrono::Utc::now());
        for uid in 500u32..800 {
            message.id = postio_model::ids::MessageId::new(0);
            message.subject = Some(format!("queued {uid}"));
            message.server.uid = Some(postio_model::Uid::new(uid));
            message.server.remote_id = Some(postio_model::RemoteId::new(format!("1:{uid}")));
            message.sync.body_state = postio_model::BodyState::HeadersOnly;
            postio_storage::repository::MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("queue a body");
        }
    }
    backend.set_latency(Duration::from_millis(200));
    engine
        .seed_backfill(mailbox.id, 400)
        .await
        .expect("seeding reads the store");

    // After the fixture rows exist, or the baseline is one this test walks past on its own.
    let before = stored_in(&database, mailbox.id).await;

    // Waited for, not slept: a sleep is stretched by `POSTIO_TEST_PATIENCE` while the mock's
    // latency is not, so on CI it outlasted the queue.
    let running = tokio::time::timeout(
        postio_test_support::scaled(Duration::from_secs(10)),
        async {
            loop {
                let progress = engine.backfill_progress().await.expect("progress");
                if progress.in_flight > 0 || progress.stored + progress.failed > 0 {
                    return progress;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await
    .expect("the backfill never started");
    assert!(
        running.pending > 100,
        "the backfill drained before the test could ask its question, so a \
         pass here would mean nothing: {running:?}"
    );

    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(arriving_message()),
        )
        .await
        .expect("the server takes delivery");

    let after = tokio::time::timeout(
        postio_test_support::scaled(Duration::from_secs(15)),
        async {
            loop {
                let count = stored_in(&database, mailbox.id).await;
                if count > before {
                    return count;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        },
    )
    .await
    .expect(
        "the delivery went unnoticed while the backfill ran — the loop never \
         gave itself back to the watcher",
    );

    assert!(
        after > before,
        "the message that arrived mid-download never landed"
    );
    drop(engine);
}

#[tokio::test]
async fn mail_that_arrives_while_the_app_is_open_turns_up() {
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, _events, _directory) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");

    // Let what the link coming up set off finish, so only the watcher can account for what follows.
    let before = settle(&database, mailbox.id).await;
    assert!(before > 0);

    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(arriving_message()),
        )
        .await
        .expect("the server takes delivery");

    let after = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let count = stored_in(&database, mailbox.id).await;
            if count > before {
                return count;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("nothing noticed the new mail — the watcher is not running");

    assert_eq!(after, before + 1, "exactly the one that arrived");
}

#[tokio::test]
async fn a_message_nobody_has_is_nothing_to_fetch() {
    let (engine, _database, _report, _events, _directory) = engine().await;

    let wanted = engine
        .request_body(postio_model::ids::MessageId::new(987_654))
        .await
        .expect("asking about a message that is not here is not a failure");

    assert!(!wanted, "there was nothing to fetch and it said so");
}

#[tokio::test]
async fn the_engine_answers_after_the_handle_is_cloned() {
    let (engine, _database, _report, _events, _directory) = engine().await;
    let second = engine.clone();

    let (first, second) = tokio::join!(engine.drain(), second.drain());
    first.expect("the first handle works");
    second.expect("and so does the clone");
}

#[tokio::test]
async fn a_connection_that_will_not_open_leaves_the_queue_where_it_is() {
    // Local-first: a drain with no session is a connection problem, never a failed operation, and
    // the row stays.
    // The fault is persistent and pre-spawn (#210, #330).
    let (engine, database, report, events, _backend, _directory) =
        engine_with(|backend| backend.fail_all(Fault::AuthFailed)).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    let message = queue_a_flag_change(&database, &report, inbox.id).await;

    let error = engine
        .drain()
        .await
        .expect_err("the credentials were refused");
    assert!(!error.message().is_empty());

    let connection = database.connect().await.expect("a connection");
    let pending = OperationQueueRepository::new(&connection)
        .pending(report.account.id, Utc::now())
        .await
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
                state: postio_core::ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
                ..
            }
        )),
        "the UI was not told the connection is the problem"
    );
}

#[tokio::test]
async fn a_failing_connection_says_which_account_the_error_is_about() {
    let (engine, _database, report, events, _backend, _directory) =
        engine_with(|backend| backend.fail_all(Fault::Io("the line went dead".to_owned()))).await;

    engine.drain().await.expect_err("the transport failed");

    let said: Vec<_> = announced(&events)
        .into_iter()
        .filter_map(|event| match event {
            Event::Error { message, account } => Some((message, account)),
            _ => None,
        })
        .collect();
    assert!(
        said.iter()
            .any(|(message, account)| !message.is_empty() && *account == Some(report.account.id)),
        "no error named the failing account: {said:?}"
    );
}

#[tokio::test]
async fn a_refused_password_blocks_and_a_new_one_unblocks() {
    // Only `retry_now` retries a refused password. The fault is persistent and pre-spawn: the
    // engine's own calls interleave, and a late fault races the supervisor (#210, #330).
    let (engine, _database, _report, events, backend, _directory) =
        engine_with(|backend| backend.fail_all(Fault::AuthFailed)).await;

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
                state: postio_core::ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
                ..
            }
        )),
        "the status line was not told"
    );

    backend.clear_faults();
    let moved = engine.retry_now().await.expect("the engine answers");
    assert!(
        !matches!(moved, Link::Blocked(_)),
        "a new password should clear the block: {moved:?}"
    );
}

#[tokio::test]
async fn a_connection_that_dies_mid_drain_parks_the_link_at_once() {
    let (engine, database, report, _events, backend, _directory) = engine_with_backend().await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    queue_a_flag_change(&database, &report, inbox.id).await;

    engine.drain().await.expect("a first pass connects");
    assert!(
        engine.link().await.expect("the engine answers").is_online(),
        "the link should be up before this test means anything"
    );

    // Fault first and persistent: the engine drains queued work on its own, and a one-shot fault
    // lost to that drain on a two-core runner.
    backend.fail_all(Fault::Disconnect);
    queue_a_flag_change(&database, &report, inbox.id).await;
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
    let (engine, _database, _report, events, _backend, _directory) = engine_with_backend().await;

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

async fn settle(database: &postio_storage::Store, mailbox: postio_model::ids::MailboxId) -> usize {
    let mut last = usize::MAX;
    for _ in 0..100 {
        let count = stored_in(database, mailbox).await;
        if count == last {
            return count;
        }
        last = count;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    last
}

/// Retries while the store is locked, since the engine writes during these tests. Takes the
/// connection by value because an async closure cannot borrow its argument here.
async fn with_store<T, F, Fut>(database: &postio_storage::Store, what: &str, work: F) -> T
where
    F: Fn(postio_storage::Checkout) -> Fut,
    Fut: std::future::Future<Output = postio_storage::Result<T>>,
{
    for _ in 0..100 {
        let connection = database.connect().await.expect("a connection");
        match work(connection).await {
            Ok(value) => return value,
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    }
    panic!("the store stayed locked: {what}");
}

async fn stored_in(
    database: &postio_storage::Store,
    mailbox: postio_model::ids::MailboxId,
) -> usize {
    with_store(database, "counting messages", async |connection| {
        let connection = &connection;
        postio_storage::repository::MessageRepository::new(connection)
            .count(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                limit: 0,
                after: None,
            })
            .await
            .map(|count| count as usize)
    })
    .await
}

fn arriving_message() -> Vec<u8> {
    b"From: Grace Hopper <grace@example.org>\r\n\
      To: Postio <postio@example.net>\r\n\
      Subject: arrived while you were looking\r\n\
      Message-ID: <arrived@example.org>\r\n\
      Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
      \r\n\
      Nobody asked for this one.\r\n"
        .to_vec()
}

fn engine_over(
    database: &postio_storage::Store,
    account: postio_model::ids::AccountId,
    backend: MockBackend,
) -> (Engine, EventStream, BlobDir) {
    engine_over_arc(database, account, Arc::new(backend))
}

fn engine_over_arc(
    database: &postio_storage::Store,
    account: postio_model::ids::AccountId,
    backend: Arc<MockBackend>,
) -> (Engine, EventStream, BlobDir) {
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();
    let engine = Engine::spawn(EngineParts {
        account,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");
    let directory = BlobDir::new(engine.clone(), directory);
    (engine, events, directory)
}

/// Locals drop in reverse order, so the blob directory would go while the engine thread is still
/// fetching and leave a tree behind (#724); the harness hands back a guard.
#[tokio::test]
async fn releasing_the_blob_directory_stops_the_engine_first() {
    let (_engine, _database, _report, _events, _backend, directory) = engine_with_backend().await;
    let watcher = _engine.clone();
    let path = directory.path().to_path_buf();
    assert!(path.exists(), "the blob directory exists to begin with");

    drop(directory);

    assert!(
        watcher.drain().await.is_err(),
        "the blob directory was removed while the engine was still running -- \
         a fetch finishing after this point recreates the tree and leaks it"
    );
    assert!(!path.exists(), "and the directory itself is gone");
}

/// Seeded messages get `uid = id`, which the mock's UIDs line up with, so a backfill test can watch
/// a body arrive.
fn server() -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: body {n}\r\n\
             Message-ID: <body-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             The bytes that had to travel to get here.\r\n"
        )
        .into_bytes()
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=10 {
        inbox = inbox.message(MockMessage::new(message(n)));
    }
    MockBackend::builder().mailbox(inbox).build()
}

/// `postio_storage::seed` assigns no UID, and `needing_backfill` will not offer a message it cannot
/// ask the server for.
async fn give_the_inbox_uids(
    database: &postio_storage::Store,
    mailbox: postio_model::ids::MailboxId,
) -> usize {
    let connection = database.connect().await.expect("a connection");
    let touched = connection
        .execute(
            "UPDATE messages SET uid = id, remote_id = '1:' || id WHERE mailbox_id = ?1",
            [mailbox.get()],
        )
        .await
        .expect("the fixture writes");
    // A fixture that quietly matched nothing would make the test blame whatever it asked next.
    assert!(
        touched > 0,
        "the fixture gave uids to no messages at all: mailbox {mailbox} is \
         empty, so nothing after this can mean what it says"
    );
    touched as usize
}

async fn queue_a_flag_change(
    database: &postio_storage::Store,
    report: &postio_storage::seed::SeedReport,
    mailbox: postio_model::ids::MailboxId,
) -> postio_model::ids::MessageId {
    with_store(database, "queueing a flag change", async |connection| {
        let connection = &connection;
        let page = postio_storage::repository::MessageRepository::new(connection)
            .page(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                limit: 1,
                after: None,
            })
            .await?;
        let message = page.first().expect("the inbox has mail").id;
        OperationQueueRepository::new(connection)
            .enqueue(
                report.account.id,
                OperationTarget::Message(message),
                &Operation::SetFlags {
                    flags: postio_model::FlagSet::from_iter([postio_model::Flag::Seen]),
                },
                Utc::now(),
            )
            .await?;
        Ok(message)
    })
    .await
}

fn announced(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test]
async fn a_draft_saved_while_connected_reaches_the_server_without_being_asked() {
    // Nothing can tell this thread a row was written, so the loop asks; before, a draft waited for
    // the next reconnection.
    let database = test_support::memory().await;
    let report = seed_small(&database, 12).await;
    let drafts_mailbox = report
        .mailbox(MailboxRole::Drafts)
        .expect("the seed has a Drafts folder");

    // Written before the engine exists: a shared in-memory SQLite answers a second writer with
    // SQLITE_LOCKED.
    let mut identity = postio_model::Identity::new(
        report.account.id,
        postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    identity.is_default = true;
    {
        let connection = database.connect().await.expect("checkout");
        postio_storage::repository::IdentityRepository::new(&connection)
            .create(&mut identity)
            .await
            .expect("create the identity");
    }

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(MockMailbox::new(&drafts_mailbox.path))
            .build(),
    );
    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !matches!(engine.link().await, Ok(Link::Online { .. })) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the engine never connected");

    let mut draft = postio_model::Draft::new(report.account.id);
    draft.to = vec![postio_model::EmailAddress::new(
        None::<String>,
        "grace@example.net",
    )];
    draft.subject = "Written while the wire was up".to_owned();
    draft.body.text = Some("A thought, mid-thought.".to_owned());
    {
        let connection = database.connect().await.expect("checkout");
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, Utc::now())
            .await
            .expect("save and queue");
    }

    // 60 s: this failed once under heavy machine load (#55); no event marks a queued operation
    // settling, and a hung engine still fails fast.
    let landed = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if let Ok(status) = backend.status(&drafts_mailbox.path).await
                && status.exists > 0
            {
                return status.exists;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the draft never reached the server on the engine's own initiative");

    assert_eq!(landed, 1, "one copy, uploaded once");
}

#[tokio::test]
async fn a_fresh_account_learns_its_folders_from_the_server() {
    let database = test_support::memory().await;
    let account = {
        let connection = database.connect().await.expect("checkout");
        let mut account = postio_model::Account::new(
            "Test",
            postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("create the account");
        account
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(MockMailbox::new("Sent Messages").attributes(["\\Sent"]))
            .mailbox(MockMailbox::new("Deleted Messages").attributes(["\\Trash"]))
            .build(),
    );
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    let folders = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let connection = database.connect().await.expect("checkout");
            let found = postio_storage::repository::MailboxRepository::new(&connection)
                .list_for_account(account.id)
                .await
                .expect("list");
            drop(connection);
            // All six: discovery writes the server's three then creates the rest, and a poll
            // between read three.
            if found.len() >= 6 {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the engine connected and never wrote down its folders");

    // Six: the three the mock lists plus the Archive, Drafts and Junk discovery creates; the point
    // is folders written down unasked.
    assert_eq!(folders.len(), 6, "{folders:?}");
    assert!(
        folders
            .iter()
            .any(|mailbox| mailbox.role == MailboxRole::Sent),
        "and the roles come from the server's attributes, so Sent is found \
         however the account spells it: {folders:?}"
    );
    drop(engine);
}

#[tokio::test]
async fn a_requested_body_does_not_wait_for_the_supervisors_first_tick() {
    // #109: a job queued before the loop's first iteration must not delay the first connection by a
    // full `POLL_INTERVAL`; 2 s is generous against ~20-40 ms.
    let (engine, database, report, _events, _directory) = engine().await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    give_the_inbox_uids(&database, inbox.id).await;

    // The message asked for must be one the mock holds (UIDs 1..=10), not merely the newest.
    let message = with_store(
        &database,
        "a message the mock actually holds",
        async |connection| {
            postio_storage::sql::one(
                &connection,
                "SELECT id FROM messages WHERE mailbox_id = ?1 AND uid BETWEEN 1 AND 10 LIMIT 1",
                bind![inbox.id.get()],
                |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
            )
            .await
        },
    )
    .await;
    let message = postio_model::ids::MessageId::new(message);

    let wanted = engine.request_body(message).await.expect("request_body");
    assert!(
        wanted,
        "there was nothing to fetch for a message the mock holds"
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let landed = with_store(&database, "reading raw_blob_id", async |connection| {
                let connection = &connection;
                postio_storage::repository::MessageRepository::new(connection)
                    .get(message)
                    .await
            })
            .await
            .expect("the message exists")
            .raw_blob_id
            .is_some();
            if landed {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect(
        "the body never arrived within 2s -- the connection attempt is \
         waiting on the ticker's first tick instead of happening at once",
    );
}

/// The body a person asked for reaches the index too, not only a backfill's (#327): the engine's
/// half is the body landing in the indexer's queue; one indexer batch then makes it findable.
#[tokio::test]
async fn a_body_the_user_asked_for_is_indexed_as_well_as_stored() {
    // The background lane is off, or both lanes chase the same message and `request_body` correctly
    // reports nothing left to fetch.
    let (engine, database, report, _events, _backend, _directory) = engine_with_backfill(
        |_| {},
        postio_sync::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    give_the_inbox_uids(&database, inbox.id).await;

    with_store(
        &database,
        "creating the search schema",
        async |connection| {
            let connection = &connection;
            postio_index::index::ensure_schema(connection)
                .await
                .expect("the search schema");
            Ok(())
        },
    )
    .await;

    let message = with_store(
        &database,
        "a message the mock actually holds",
        async |connection| {
            postio_storage::sql::one(
                &connection,
                "SELECT id FROM messages WHERE mailbox_id = ?1 AND uid BETWEEN 1 AND 10 LIMIT 1",
                bind![inbox.id.get()],
                |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
            )
            .await
        },
    )
    .await;
    let message = postio_model::ids::MessageId::new(message);

    // `body_search` is NULL until `index_body` has run; the failure must propagate, since
    // defaulting once turned a missing column into "not indexed yet".
    let body_is_indexed = async |id: i64| {
        with_store(
            &database,
            "looking for the indexed body",
            async |connection| {
                postio_storage::sql::exists(
                    &connection,
                    "SELECT 1 FROM message_search_bodies WHERE content_id = (SELECT content_id FROM messages WHERE id = ?1)",
                    bind![id],
                )
                .await
            },
        )
        .await
    };
    assert!(
        !body_is_indexed(message.get()).await,
        "the body is not local yet, so nothing about it can be in the index"
    );

    assert!(
        engine.request_body(message).await.expect("request_body"),
        "there was nothing to fetch for a message the mock holds"
    );

    let pending = async |id: i64| {
        with_store(
            &database,
            "asking the indexer's queue",
            async |connection| {
                let queue = postio_index::index::messages_missing_body_text(&connection, 100, None)
                    .await
                    .expect("the indexer's queue");
                Ok(queue.iter().any(|row| row.id == id))
            },
        )
        .await
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if pending(message.get()).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect(
        "the body the user opened never landed as a body the indexer would \
         pick up, so search covers only whatever the background backfill \
         happened to have fetched (#327)",
    );
    assert!(
        !body_is_indexed(message.get()).await,
        "the fetch wrote the search row itself, on the sync lane"
    );

    with_store(&database, "one indexer batch", async |connection| {
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let queue = postio_index::index::messages_missing_body_text(&connection, 100, None)
            .await
            .expect("the indexer's queue");
        for candidate in queue {
            let id = candidate.id;
            let stored = messages
                .body(postio_model::ids::MessageId::new(id))
                .await?
                .unwrap_or_default();
            let body = postio_model::MessageBody {
                text: stored.text,
                html: stored.html,
            };
            postio_index::index::index_body_of(&connection, id, &body)
                .await
                .expect("index a body");
        }
        Ok(())
    })
    .await;
    assert!(
        body_is_indexed(message.get()).await,
        "one indexer batch over the queue did not reach the body the user asked for"
    );
}

/// Ten messages and a batch of three rather than 40,000 and 200: the property needs a mailbox
/// bigger than one batch.
async fn engine_seeding_in_batches(
    messages: u32,
    seed_batch: u32,
) -> (
    Engine,
    postio_storage::Store,
    postio_model::ids::MailboxId,
    Arc<MockBackend>,
    BlobDir,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
    drop(connection);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let mut mailbox = MockMailbox::new("INBOX");
    for n in 1..=messages {
        mailbox = mailbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 To: Postio <postio@example.net>\r\n\
                 Subject: body {n}\r\n\
                 Message-ID: <batch-{n}@example.com>\r\n\
                 Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                 \r\n\
                 The bytes that had to travel to get here.\r\n"
            )
            .into_bytes(),
        ));
    }

    let backend = Arc::new(MockBackend::builder().mailbox(mailbox).build());
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: postio_sync::backfill::BackfillPolicy {
            // The user is never active in a test, but leaving the pause on would depend on that.
            pause_when_active: false,
            seed_batch,
            ..Default::default()
        },
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    let directory = BlobDir::new(engine.clone(), directory);
    (engine, database, inbox.id, backend, directory)
}

async fn until<F, Fut>(done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if done().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .is_ok()
}

async fn headers_in(
    database: &postio_storage::Store,
    mailbox: postio_model::ids::MailboxId,
) -> i64 {
    with_store(database, "counting headers", async |connection| {
        let connection = &connection;
        postio_storage::sql::one(
            connection,
            "SELECT count(*) FROM messages WHERE mailbox_id = ?1",
            bind![mailbox.get()],
            |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
        )
        .await
    })
    .await
}

async fn bodies_local(
    database: &postio_storage::Store,
    mailbox: postio_model::ids::MailboxId,
) -> i64 {
    with_store(database, "counting local bodies", async |connection| {
        let connection = &connection;
        postio_storage::sql::one(
            connection,
            "SELECT count(*) FROM messages WHERE mailbox_id = ?1 AND body_state = 'full'",
            bind![mailbox.get()],
            |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
        )
        .await
    })
    .await
}

/// #318: coverage, not throughput. Nobody opens anything; every eligible body of a mailbox bigger
/// than one batch must end up local.
#[tokio::test]
async fn a_mailbox_larger_than_one_seed_is_covered_without_anyone_opening_it() {
    const MESSAGES: u32 = 10;
    const BATCH: u32 = 3;
    let (engine, database, inbox, _backend, _directory) =
        engine_seeding_in_batches(MESSAGES, BATCH).await;

    let covered = until(async || bodies_local(&database, inbox).await == i64::from(MESSAGES)).await;

    let local = bodies_local(&database, inbox).await;
    assert!(
        covered,
        "{local} of {MESSAGES} bodies are local and the backfill has stopped. \
         The queue is seeded once per folder at startup and nothing ever tops \
         it up, so everything below the first batch of {BATCH} waits to be \
         opened (#318)."
    );

    drop(engine);
}

/// #318, the other half: a message arriving after startup is backfilled, which needs the top-up
/// after a drained queue.
#[tokio::test]
async fn mail_arriving_after_startup_is_backfilled_without_being_opened() {
    const MESSAGES: u32 = 4;
    let (engine, database, inbox, backend, _directory) =
        engine_seeding_in_batches(MESSAGES, 2).await;

    assert!(
        until(async || bodies_local(&database, inbox).await == i64::from(MESSAGES)).await,
        "the mailbox it started with never finished, so nothing below is \
         about mail that arrived later"
    );

    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(
                b"From: Ada Lovelace <ada@example.com>\r\n\
                  To: Postio <postio@example.net>\r\n\
                  Subject: after the fact\r\n\
                  Message-ID: <late-1@example.com>\r\n\
                  Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
                  \r\n\
                  Delivered while the client was already running.\r\n"
                    .to_vec(),
            ),
        )
        .await
        .expect("the mock accepts a delivery");

    let arrived =
        until(async || bodies_local(&database, inbox).await == i64::from(MESSAGES) + 1).await;
    assert!(
        arrived,
        "a message delivered after startup has {} of {} bodies local: it got \
         a row and no body, and would have waited to be opened (#318)",
        bodies_local(&database, inbox).await,
        MESSAGES + 1
    );

    drop(engine);
}

/// The top-up must not get around the policy: with the lane paused it would pull in an archive, and
/// a message over `max_body_bytes` must be counted skipped once.
///
/// POSTIO-MEASUREMENT: it sleeps two seconds for a late re-seed to have its chance, so
/// it runs on the nightly timer.
#[tokio::test]
async fn the_top_up_does_not_outrank_the_policy_it_runs_under() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;
    drop(connection);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let mut mailbox = MockMailbox::new("INBOX");
    for n in 1..=6u32 {
        mailbox = mailbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 Subject: big {n}\r\n\
                 Message-ID: <big-{n}@example.com>\r\n\
                 Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                 \r\n\
                 {}\r\n",
                "x".repeat(4096)
            )
            .into_bytes(),
        ));
    }

    let backend = Arc::new(MockBackend::builder().mailbox(mailbox).build());
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: postio_sync::backfill::BackfillPolicy {
            max_body_bytes: Some(64),
            pause_when_active: false,
            seed_batch: 2,
            ..Default::default()
        },
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    assert!(
        until(async || headers_in(&database, inbox).await == 6).await,
        "the headers never synced, so nothing below is about the backfill"
    );

    // A delivery makes the test bite: the sync re-runs the top-up and every over-cap row comes back
    // out of `needing_backfill`, each to be left alone rather than skipped again.
    backend
        .append(
            "INBOX",
            &postio_account::backend::AppendMessage::new(
                format!(
                    "From: Ada Lovelace <ada@example.com>\r\n\
                     Subject: big 7\r\n\
                     Message-ID: <big-7@example.com>\r\n\
                     Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
                     \r\n\
                     {}\r\n",
                    "x".repeat(4096)
                )
                .into_bytes(),
            ),
        )
        .await
        .expect("the mock accepts a delivery");

    // Waited for in the store: polling the engine would starve the work under test, since each job
    // must be served before it syncs.
    assert!(
        until(async || headers_in(&database, inbox).await == 7).await,
        "the delivery never reached the store, so the re-seed it triggers \
         never happened"
    );
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    assert_eq!(
        bodies_local(&database, inbox).await,
        0,
        "the background lane fetched a body over `max_body_bytes`, so the \
         top-up is going around the policy rather than under it"
    );

    let progress = engine.backfill_progress().await.expect("progress");
    assert_eq!(
        progress.skipped, 7,
        "seven messages, {} skips: an over-cap message is counted again on \
         every re-seed, so the status line's numbers drift upward for ever \
         and the top-up has nothing telling it when to stop",
        progress.skipped
    );
    assert_eq!(
        progress.stored, 0,
        "nothing should have been stored by the background lane"
    );

    drop(engine);
}

const PDF: &[u8] = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
const PDF_BASE64: &str = "JVBERi0xLjQKJeLjz9MK\r\n";

fn server_with_an_attachment() -> MockBackend {
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 27)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", PDF.len() as u64)
                .with_encoding("base64")
                .with_filename("statement.pdf"),
        ],
    );
    let raw = "From: Ada Lovelace <ada@example.com>\r\n\
               To: Postio <postio@example.net>\r\n\
               Subject: Your statement\r\n\
               Message-ID: <statement@example.com>\r\n\
               Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
               Content-Type: multipart/mixed; boundary=b\r\n\
               \r\n\
               --b\r\n\
               Content-Type: text/plain; charset=utf-8\r\n\
               \r\n\
               Your statement is attached.\r\n\
               --b--\r\n";
    MockBackend::builder()
        .mailbox(
            MockMailbox::new("INBOX").message(
                MockMessage::new(raw.as_bytes().to_vec())
                    .with_structure(structure)
                    .with_part("1", &b"Your statement is attached."[..])
                    .with_part("2", PDF_BASE64.as_bytes()),
            ),
        )
        .build()
}

async fn engine_over_a_real_sync(
    backfill: postio_sync::BackfillPolicy,
) -> (
    Engine,
    postio_storage::Store,
    postio_model::AccountId,
    BlobDir,
    test_support::TempStore,
) {
    let (engine, database, account, directory, temp, _events) =
        telling_engine_over_a_real_sync(backfill).await;
    (engine, database, account, directory, temp)
}

async fn telling_engine_over_a_real_sync(
    backfill: postio_sync::BackfillPolicy,
) -> (
    Engine,
    postio_storage::Store,
    postio_model::AccountId,
    BlobDir,
    test_support::TempStore,
    postio_core::bridge::EventStream,
) {
    let database = test_support::temp().await;
    let account = {
        let connection = database.connect().await.expect("checkout");
        test_support::account(&connection).await
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: (*database).clone(),
        blobs,
        backend: Arc::new(server_with_an_attachment()),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill,
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");
    let id = account.id;
    // `database` goes back too so the temp directory stays on disk while either clone is used
    // (#605).
    let cloned = (*database).clone();
    let directory = BlobDir::new(engine.clone(), directory);
    (engine, cloned, id, directory, database, events)
}

async fn until_some<T, F, Fut>(database: &postio_storage::Store, what: &str, look: F) -> T
where
    F: Fn(postio_storage::Checkout) -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Ok(connection) = database.connect().await
                && let Some(found) = look(connection).await
            {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never saw {what}"))
}

async fn the_synced_message(
    database: &postio_storage::Store,
    account: postio_model::AccountId,
) -> postio_model::MessageId {
    until_some(
        database,
        "the message reach the store",
        async |connection| {
            let connection = &connection;
            let mailboxes = postio_storage::repository::MailboxRepository::new(connection)
                .list_for_account(account)
                .await
                .ok()?;
            let inbox = mailboxes.iter().find(|mailbox| mailbox.path == "INBOX")?;
            postio_storage::repository::MessageRepository::new(connection)
                .page(&postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(inbox.id),
                    limit: 1,
                    after: None,
                })
                .await
                .ok()?
                .first()
                .map(|row| row.id)
        },
    )
    .await
}

async fn attachment_blob(
    database: &postio_storage::Store,
    message: postio_model::MessageId,
) -> Option<postio_model::BlobId> {
    let connection = database.connect().await.ok()?;
    postio_storage::repository::MessageRepository::new(&connection)
        .get(message)
        .await
        .ok()??
        .attachments
        .first()?
        .blob_id
        .clone()
}

#[tokio::test]
async fn the_default_policy_syncs_the_words_and_leaves_the_payload_on_the_server() {
    let (engine, database, account, _directory, _temp_database) =
        engine_over_a_real_sync(postio_sync::BackfillPolicy::default()).await;
    let message = the_synced_message(&database, account).await;

    let state = until_some(&database, "the text land", async |connection| {
        let connection = &connection;
        let row = postio_storage::repository::MessageRepository::new(connection)
            .get(message)
            .await
            .ok()??;
        (row.sync.body_state != postio_model::BodyState::HeadersOnly).then_some(row.sync.body_state)
    })
    .await;

    assert_eq!(
        state,
        postio_model::BodyState::Partial,
        "text local, payload not"
    );
    assert_eq!(
        attachment_blob(&database, message).await,
        None,
        "nothing speculative pulls a payload under the default policy"
    );
    drop(engine);
}

#[tokio::test]
async fn opening_an_attachment_asks_the_engine_for_that_one_section() {
    let (engine, database, account, _directory, _temp_database, events) =
        telling_engine_over_a_real_sync(postio_sync::BackfillPolicy::default()).await;
    let message = the_synced_message(&database, account).await;

    // Wait for the text first: asking before the header sync wrote the attachment rows proves
    // nothing.
    until_some(&database, "the text land", async |connection| {
        let connection = &connection;
        let row = postio_storage::repository::MessageRepository::new(connection)
            .get(message)
            .await
            .ok()??;
        (row.sync.body_state == postio_model::BodyState::Partial).then_some(())
    })
    .await;

    let asked = engine
        .request_payloads(message, vec!["2".to_owned()])
        .await
        .expect("the engine answers");
    assert_eq!(
        asked,
        postio_model::fetch::PayloadAsk::Queued,
        "there is a payload on the server to ask for"
    );

    // The state, not the blob: `fetch_payloads` writes the blob id and `body_state` separately, so
    // a read between sees `Partial` for a payload already on disk.
    let row = until_some(
        &database,
        "the payload land and settle",
        async |connection| {
            let connection = &connection;
            let row = postio_storage::repository::MessageRepository::new(connection)
                .get(message)
                .await
                .ok()??;
            (row.sync.body_state == postio_model::BodyState::Full).then_some(row)
        },
    )
    .await;

    assert!(
        row.attachments[0].is_downloaded(),
        "every part is local now"
    );
    assert!(
        attachment_blob(&database, message).await.is_some(),
        "and the blob it names is in the store"
    );
    let mut heard = Vec::new();
    while let Some(event) = events.try_next() {
        if let postio_core::Event::BodyLoaded {
            message: loaded,
            sections,
            ..
        } = event
            && loaded == message
        {
            heard.push(sections);
        }
    }
    assert!(
        heard.contains(&postio_model::fetch::Sections::Parts(vec!["2".to_owned()])),
        "the part's landing was never said by section: {heard:?}"
    );
    assert!(
        heard.contains(&postio_model::fetch::Sections::Text),
        "the words landed first, said as words: {heard:?}"
    );
    assert_eq!(
        engine
            .request_payloads(message, vec!["2".to_owned()])
            .await
            .expect("the engine answers"),
        postio_model::fetch::PayloadAsk::Local
    );
    drop(engine);
}

#[tokio::test]
async fn eager_drains_the_payloads_with_nobody_asking() {
    let (engine, database, account, _directory, _temp_database) =
        engine_over_a_real_sync(postio_sync::BackfillPolicy {
            attachments: postio_sync::AttachmentPolicy::Eager,
            ..Default::default()
        })
        .await;
    let message = the_synced_message(&database, account).await;

    // Wait for the state, not the blob: the blob id and `body_state` are two writes, and reading
    // the second right after the first is a coin toss.
    let settled = until_some(
        &database,
        "the payload land and settle",
        async |connection| {
            let connection = &connection;
            let row = postio_storage::repository::MessageRepository::new(connection)
                .get(message)
                .await
                .ok()??;
            (row.sync.body_state == postio_model::BodyState::Full).then_some(row)
        },
    )
    .await;

    assert!(
        settled.attachments[0].is_downloaded(),
        "`full` means the payload is here, not merely asked for"
    );
    assert!(
        attachment_blob(&database, message).await.is_some(),
        "and the blob it names is in the store"
    );
    drop(engine);
}

fn a_server_with_many_folders() -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             Subject: note {n}\r\n\
             Message-ID: <note-{n}@example.com>\r\n\
             \r\n\
             The bytes that had to travel to get here.\r\n"
        )
        .into_bytes()
    };
    let mut builder = MockBackend::builder();
    for folder in ["INBOX", "Archive", "Sent", "Drafts", "Trash", "Junk"] {
        let mut mailbox = MockMailbox::new(folder);
        for n in 1..=8 {
            mailbox = mailbox.message(MockMessage::new(message(n)));
        }
        builder = builder.mailbox(mailbox);
    }
    builder.build()
}

async fn within<F, Fut>(bound: std::time::Duration, done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + postio_test_support::scaled(bound);
    while std::time::Instant::now() < deadline {
        if done().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    done().await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_action_does_not_wait_out_a_sync_wave() {
    // #944: the backfill loops and idle watcher yield to queued work but the sync wave did not, so
    // an action taken mid-wave waited for every folder. Latency makes the wave long enough to see
    // into.
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();
    let backend = Arc::new(a_server_with_many_folders());
    backend.set_latency(std::time::Duration::from_millis(120));

    let _engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    // Wait for the wave to be under way so the action is enqueued during one: before the first
    // SELECT the engine is still connecting.
    assert!(
        within(std::time::Duration::from_secs(20), async || backend.calls()
            >= 8)
        .await,
        "the engine never started syncing folders, so there is no wave to interrupt"
    );

    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    queue_a_flag_change(&database, &report, inbox.id).await;

    let drained = within(std::time::Duration::from_secs(3), async || {
        let Ok(connection) = database.connect().await else {
            return false;
        };
        OperationQueueRepository::new(&connection)
            .pending(report.account.id, Utc::now())
            .await
            .is_ok_and(|due| due.is_empty())
    })
    .await;
    assert!(
        drained,
        "a flag set during a sync wave never reached the server. The wave \
         yields to a job but not to queued work, so a send, a mark-read or a \
         move waits out every folder still in `to_sync` -- which after a link \
         transition is all of them. Backend calls so far: {}",
        backend.calls()
    );
}

/// POSTIO-MEASUREMENT: it sleeps three seconds, twice, for the first sync to settle
/// into the watch and for idleness to show, so it runs on the nightly timer.
#[tokio::test]
async fn an_idle_engine_does_not_poll_the_queue() {
    if postio_test_support::alone!("an_idle_engine_does_not_poll_the_queue") {
        return;
    }
    // The engine once polled the store every 500 ms and its channel every 50 ms on an idle app; a
    // person's action and a job now wake it. No background body lane: a backfill is something to
    // do.
    let (engine, database, report, _events, _backend, _directory) = engine_with_backfill(
        |_| {},
        postio_sync::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let before = postio_runtime::engine::idle_wakeups();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let wakeups = postio_runtime::engine::idle_wakeups() - before;
    assert!(
        wakeups <= 4,
        "an idle engine's waits woke {wakeups} times on their own in three seconds"
    );

    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed has an inbox")
        .id;
    {
        let (connection, _permit) = database.interactive_write().await.expect("a write");
        let message = postio_storage::repository::MessageRepository::new(&connection)
            .page(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(inbox),
                limit: 1,
                after: None,
            })
            .await
            .expect("a page")
            .first()
            .expect("the inbox has mail")
            .id;
        OperationQueueRepository::new(&connection)
            .enqueue(
                report.account.id,
                OperationTarget::Message(message),
                &Operation::SetFlags {
                    flags: postio_model::FlagSet::from_iter([postio_model::Flag::Seen]),
                },
                Utc::now(),
            )
            .await
            .expect("queued");
    }
    let started = std::time::Instant::now();
    loop {
        let connection = database.connect().await.expect("a connection");
        let due = OperationQueueRepository::new(&connection)
            .pending(report.account.id, Utc::now())
            .await
            .expect("the queue");
        if due.is_empty() {
            break;
        }
        assert!(
            started.elapsed() < postio_test_support::scaled(std::time::Duration::from_secs(2)),
            "a queued action waited out the fallback tick instead of waking the engine"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    drop(engine);
}

#[tokio::test]
async fn a_finished_pass_over_an_empty_mailbox_says_a_sync_completed() {
    // A pass with nothing to fetch must still reach `done == total`, so a frontend can tell an
    // empty inbox from one not yet reached.
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let empty = MockBackend::builder()
        .mailbox(postio_account::backend::MockMailbox::new("INBOX"))
        .build();
    let (engine, events, _directory) = engine_over(&database, account.id, empty);

    engine.sync(mailbox.id).await.expect("a sync pass");

    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::SyncProgress {
                done: 0,
                total: 0,
                ..
            }
        )),
        "the pass finished and the stream never said so"
    );
}

#[tokio::test]
async fn a_pass_that_failed_is_not_announced_as_a_completed_sync() {
    // An interrupted or failed first pass must not read as finished, or the inbox it never filled
    // would be called empty.
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .await
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());
    engine.sync(mailbox.id).await.expect("a first pass");
    let first_done = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if announced(&events).iter().any(|event| {
                matches!(
                    event,
                    Event::SyncProgress {
                        done: 0,
                        total: 0,
                        ..
                    }
                )
            }) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(first_done.is_ok(), "the first pass never said it finished");
    backend.fail_all(Fault::Rejected("no".to_owned()));
    engine
        .retry_now()
        .await
        .expect("a job no wave answers beside its lanes is served once every lane settled");
    let _ = announced(&events);

    engine.sync(mailbox.id).await.expect_err("the pass failed");

    let seen = announced(&events);
    assert!(
        seen.iter().any(|event| matches!(
            event,
            Event::MailboxSyncChanged { mailbox: failed, failure: Some(_), .. } if *failed == mailbox.id
        )),
        "the second pass never failed the folder: {seen:?}"
    );
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            Event::SyncProgress {
                done: 0,
                total: 0,
                ..
            }
        )),
        "a failed pass was announced as a completed sync: {seen:?}"
    );
}

async fn said_until(
    events: &EventStream,
    seen: &mut Vec<Event>,
    wanted: impl Fn(&Event) -> bool,
) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            seen.extend(announced(events));
            if seen.iter().any(&wanted) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn a_folder_the_server_will_not_open_is_marked_until_a_pass_of_it_completes() {
    // A failed pass's folder mark outlives the wave and only its own next pass clears it (#1731).
    let database = test_support::memory().await;
    let account =
        postio_storage::test_support::account(&database.connect().await.expect("a connection"))
            .await;
    let connection = database.connect().await.expect("a connection");
    let inbox = postio_storage::test_support::mailbox(&connection, &account, "INBOX").await;
    let sent = postio_storage::test_support::mailbox(&connection, &account, "Sent").await;
    drop(connection);
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(MockMailbox::new("Sent"))
            .build(),
    );
    backend.refuse_select("Sent", "Mailbox is locked");
    let (engine, events, _directory) = engine_over_arc(&database, account.id, backend.clone());
    let mark = |mailbox: postio_model::MailboxId, failed: bool| {
        move |event: &Event| {
            matches!(
                event,
                Event::MailboxSyncChanged { mailbox: marked, failure, .. }
                    if *marked == mailbox && failure.is_some() == failed
            )
        }
    };
    let mut seen = Vec::new();

    engine.sync(inbox.id).await.expect("INBOX opens");
    engine.sync(sent.id).await.expect_err("Sent does not");
    assert!(
        said_until(&events, &mut seen, mark(sent.id, true)).await,
        "the failed folder was never marked: {seen:?}"
    );
    let reason = seen.iter().find_map(|event| match event {
        Event::MailboxSyncChanged {
            failure: Some(reason),
            ..
        } => Some(reason.clone()),
        _ => None,
    });
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Mailbox is locked")),
        "the mark says why: {reason:?}"
    );
    assert!(
        !seen.iter().any(mark(inbox.id, true)),
        "INBOX synced: {seen:?}"
    );
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            Event::ConnectionChanged { state, .. }
                if *state != postio_core::ConnectionState::Online
                    && *state != postio_core::ConnectionState::Connecting
        )),
        "one folder's refusal is not the link's: {seen:?}"
    );

    backend.allow_select("Sent");
    engine.sync(sent.id).await.expect("Sent opens now");
    assert!(
        said_until(&events, &mut seen, mark(sent.id, false)).await,
        "the folder's completed pass never cleared it: {seen:?}"
    );
    let last = seen
        .iter()
        .rev()
        .find(|event| matches!(event, Event::MailboxSyncChanged { .. }));
    assert!(
        last.is_some_and(mark(sent.id, false)),
        "the last word about Sent is that it synced: {last:?}"
    );
}
