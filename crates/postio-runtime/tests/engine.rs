//! The sync engine, against a mock server.
//!
//! Nothing here touches the network: the backend is
//! `postio_imap::backend::MockBackend`, and no SMTP transport is given, so
//! nothing is ever dialled.

use std::sync::Arc;

use chrono::Utc;
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_imap::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_model::MailboxRole;
use postio_model::operation::{Operation, OperationTarget};
use postio_runtime::engine::{Engine, EngineParts, Link, NetworkSource, NetworkState};
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

    let backend = Arc::new(server());
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
        watch: Default::default(),
        network: NetworkSource::Ignored,
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

    engine.drain().await.expect("a drain pass");

    // What matters is that the row is settled, not which pass settled it: the
    // engine drains on its own when the link comes up, so this explicit one
    // may well find the queue already empty. Before any of this existed, the
    // row sat pending for ever.
    let still_pending = with_store(&database, "reading the queue", |connection| {
        OperationQueueRepository::new(connection).pending(report.account.id, Utc::now())
    });
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
async fn a_seeded_body_is_actually_fetched() {
    // postio-26c's real gap. `seed` queued bodies and nothing ever claimed
    // one, so every message stayed headers-only for ever. The loop has to
    // take a claim, fetch it, and report what became of it — otherwise the
    // queue grows and the reading pane shows nothing.
    let (engine, database, report, _events) = engine();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    give_the_inbox_uids(&database, inbox.id);

    let queued = engine
        .seed_backfill(inbox.id, 10)
        .await
        .expect("seeding reads the store");
    assert!(queued > 0, "the seed left nothing worth fetching");

    // Give the loop a moment to claim and settle what it was handed.
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
    // Issue #74. `Engine::backfill_progress` could always answer this and
    // nothing ever called it, so the longest phase of a first sync -- the
    // bodies, not the list -- reached the frontend as no event at all. The
    // sidebar drew `idle`, which is worse than silence: a user watching
    // `idle` while the log fetches bodies concludes it is stuck.
    let (engine, database, report, events) = engine();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");

    give_the_inbox_uids(&database, inbox.id);

    let queued = engine
        .seed_backfill(inbox.id, 10)
        .await
        .expect("seeding reads the store");
    assert!(queued > 0, "the seed left nothing worth fetching");

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

    // The numbers have to be usable, not merely present: a status line needs
    // a count that climbs and a denominator that does not lie.
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
    // The queue announces itself when it is filled, not after the first body
    // has been and gone. Seeding is when the denominator becomes known, and
    // a line that appears only once a fetch has completed is silent for
    // exactly the stretch someone is most likely to be watching it.
    assert_eq!(
        reports[0].0, 0,
        "the first report already had bodies settled, so nothing was said \
         when the queue was filled: {reports:?}"
    );

    // A denominator that only ever equals the numerator is not a
    // denominator. At least one report has to show work still outstanding,
    // or the line can never say anything except "finished".
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
    // postio-uif. `sync_mailbox` and `resync_mailbox` were written, tested
    // and never called, so the local store only ever held what something
    // else had put there and a fresh account stayed empty for ever.
    let database = test_support::memory();
    let account =
        postio_storage::test_support::account(&database.connection().expect("a connection"));
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .expect("the folder is created");
        mailbox
    };
    let (engine, events) = engine_over(&database, account.id, server());

    let summary = engine.sync(mailbox.id).await.expect("a sync pass");
    assert!(
        summary.inserted > 0,
        "the server had mail and none of it arrived: {summary:?}"
    );

    let stored = stored_in(&database, mailbox.id);
    assert_eq!(
        stored, summary.inserted,
        "the pass said it wrote rows the store does not have"
    );

    // And the list showing that folder has to be told it moved.
    assert!(
        announced(&events).iter().any(|event| matches!(
            event,
            Event::MessageListChanged { mailbox: changed, .. } if *changed == mailbox.id
        )),
        "nothing told the open list its mailbox had changed"
    );

    // postio-du6: a first sync inserted ten messages, and none of them are
    // new mail in the sense a desktop notification means -- they are the
    // account's whole history arriving at once. `Event::NewMail` firing here
    // would be a notification storm on the very first run.
    assert!(
        !announced(&events)
            .iter()
            .any(|event| matches!(event, Event::NewMail { .. })),
        "an initial sync must never announce new mail"
    );
}

#[tokio::test]
async fn a_resync_that_finds_new_mail_announces_it() {
    // postio-du6: `Event::NewMail` existed, was consumed by
    // `postio_gtk::feed`, and nothing ever emitted it -- the trigger a
    // desktop notification needs simply never fired.
    let database = test_support::memory();
    let account =
        postio_storage::test_support::account(&database.connection().expect("a connection"));
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    // The bootstrap pass's own events are not what this test is about.
    let _ = announced(&events);

    backend
        .append(
            "INBOX",
            &postio_imap::backend::AppendMessage::new(arriving_message()),
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

#[tokio::test]
async fn mail_arriving_on_a_resync_is_an_arrival_rather_than_a_reload() {
    // Issue #72: the list flickered on every sync tick. Both halves of the
    // reason are here rather than in the view.
    //
    // A pass that only delivered new mail emitted `NewMail` *and*
    // `MessageListChanged`. The view does the right thing with each of them
    // separately -- `NewMail` is a prepend that keeps every row widget, its
    // selection and its scroll position; `MessageListChanged` is
    // `MessageList::invalidate`, which is documented as the blunt instrument
    // for when the *order* moved and tells GTK every row was removed and
    // re-added. Sending both means the blunt one runs, and the cheap one is
    // an optimisation that never gets to happen.
    //
    // The engine knows which this was: `arrived` accounts for every insert,
    // and nothing was updated or re-threaded. A change that can describe
    // itself precisely must not also ask for a reload.
    let database = test_support::memory();
    let account =
        postio_storage::test_support::account(&database.connection().expect("a connection"));
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, events) = engine_over_arc(&database, account.id, backend.clone());

    engine.sync(mailbox.id).await.expect("a first sync");
    // The bootstrap pass reloads on purpose -- see the test above. What this
    // one is about starts after it.
    let _ = announced(&events);

    backend
        .append(
            "INBOX",
            &postio_imap::backend::AppendMessage::new(arriving_message()),
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
    // The last piece of postio-26c: a sync is exactly when the set of
    // messages missing a body changes, so it is exactly when the backfill is
    // worth seeding again. Seeding anywhere else means fetching bodies for
    // mail that has not arrived, or not fetching them for mail that has.
    let database = test_support::memory();
    let account =
        postio_storage::test_support::account(&database.connection().expect("a connection"));
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .expect("the folder is created");
        mailbox
    };
    let (engine, _events) = engine_over(&database, account.id, server());

    // Nothing is queued before the mail exists.
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

#[tokio::test]
async fn mail_that_arrives_while_the_app_is_open_turns_up() {
    // postio-e4n. The engine synced when the link came up and never again, so
    // a Postio left open all afternoon showed nothing that arrived during it
    // — which for a mail client is the whole job.
    let database = test_support::memory();
    let account =
        postio_storage::test_support::account(&database.connection().expect("a connection"));
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let mut mailbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut mailbox)
            .expect("the folder is created");
        mailbox
    };
    let backend = Arc::new(server());
    let (engine, _events) = engine_over_arc(&database, account.id, backend.clone());

    // A first pass, so the mailbox has sync state and the watcher is on it.
    engine.sync(mailbox.id).await.expect("a first sync");

    // Then let everything the connection coming up set off finish, so what
    // this test observes afterwards can only be the watcher. Without this it
    // would be racing the sync that a fresh link queues for every mailbox.
    let before = settle(&database, mailbox.id).await;
    assert!(before > 0);

    // Now a message lands on the server, with nobody asking for it.
    // `append` is how the mock's INBOX gains one; a real server would have
    // been handed it by somebody else's SMTP.
    backend
        .append(
            "INBOX",
            &postio_imap::backend::AppendMessage::new(arriving_message()),
        )
        .await
        .expect("the server takes delivery");

    let after = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let count = stored_in(&database, mailbox.id);
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
                state: postio_core::ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
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

/// Wait until the store stops changing, and say where it settled.
async fn settle(
    database: &postio_storage::Database,
    mailbox: postio_model::ids::MailboxId,
) -> usize {
    let mut last = usize::MAX;
    for _ in 0..100 {
        let count = stored_in(database, mailbox);
        if count == last {
            return count;
        }
        last = count;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    last
}

/// Run `work` against the store, retrying while it is locked.
///
/// The engine writes while these tests read and write, and an in-memory
/// database uses SQLite's *shared cache* — where meeting a writer gives
/// `SQLITE_LOCKED`, which `busy_timeout` does not cover, unlike the
/// `SQLITE_BUSY` a real installation would see. A file in WAL mode never hits
/// this; it is the price of a database that costs nothing to create, and it
/// belongs in the tests rather than in the pragmas.
fn with_store<T>(
    database: &postio_storage::Database,
    what: &str,
    work: impl Fn(&postio_storage::PooledConnection) -> postio_storage::Result<T>,
) -> T {
    for _ in 0..100 {
        let connection = database.connection().expect("a connection");
        match work(&connection) {
            Ok(value) => return value,
            Err(_) => {
                drop(connection);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    panic!("the store stayed locked: {what}");
}

/// How many messages the local store holds for `mailbox`.
fn stored_in(database: &postio_storage::Database, mailbox: postio_model::ids::MailboxId) -> usize {
    with_store(database, "counting messages", |connection| {
        postio_storage::repository::MessageRepository::new(connection)
            .count(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                limit: 0,
                after: None,
            })
            .map(|count| count as usize)
    })
}

/// One more message, of the kind `server` holds.
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

/// An engine over `database`, for a test that builds its own store.
fn engine_over(
    database: &postio_storage::Database,
    account: postio_model::ids::AccountId,
    backend: MockBackend,
) -> (Engine, EventStream) {
    engine_over_arc(database, account, Arc::new(backend))
}

/// As [`engine_over`], keeping the mock so a test can change what the server
/// holds while the engine is running.
fn engine_over_arc(
    database: &postio_storage::Database,
    account: postio_model::ids::AccountId,
    backend: Arc<MockBackend>,
) -> (Engine, EventStream) {
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, events) = event_channel();
    let engine = Engine::spawn(EngineParts {
        account,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
    })
    .expect("the engine starts");
    (engine, events)
}

/// A mock server holding an INBOX with mail in it.
///
/// The seeded database's messages get `uid = id`, so the mock's own UIDs —
/// handed out from 1 in the order given — line up with them. That is what
/// lets a backfill test watch a body actually arrive rather than only watch
/// it fail.
fn server() -> MockBackend {
    // Written out rather than taken from the corpus: the corpus loader is
    // behind a postio-model feature, and what this needs is three messages
    // with bodies, not three realistic ones. Reserved domains, per CLAUDE.md.
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

/// Give the seeded messages server UIDs.
///
/// `postio_storage::seed` writes bodies as `NotFetched` but assigns no UID —
/// it exists to fill a screenshot, not to stand in for a synced mailbox — and
/// `needing_backfill` will not offer a message it cannot ask the server for.
/// A backfill test has to supply that itself.
fn give_the_inbox_uids(database: &postio_storage::Database, mailbox: postio_model::ids::MailboxId) {
    let connection = database.connection().expect("a connection");
    connection
        .execute(
            "UPDATE messages SET uid = id WHERE mailbox_id = ?1",
            [mailbox.get()],
        )
        .expect("the fixture writes");
}

/// Queue one flag change against the newest message in `mailbox`.
fn queue_a_flag_change(
    database: &postio_storage::Database,
    report: &postio_storage::seed::SeedReport,
    mailbox: postio_model::ids::MailboxId,
) -> postio_model::ids::MessageId {
    with_store(database, "queueing a flag change", |connection| {
        let page = postio_storage::repository::MessageRepository::new(connection).page(
            &postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(mailbox),
                limit: 1,
                after: None,
            },
        )?;
        let message = page.first().expect("the inbox has mail").id;
        OperationQueueRepository::new(connection).enqueue(
            report.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: postio_model::FlagSet::from_iter([postio_model::Flag::Seen]),
            },
            Utc::now(),
        )?;
        Ok(message)
    })
}

/// What the engine announced, drained without blocking.
fn announced(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test]
async fn a_draft_saved_while_connected_reaches_the_server_without_being_asked() {
    // Nothing in the app can tell this thread that a row was written — the
    // composer autosaves on the GTK thread and the queue is just a table. So
    // the loop asks, and a draft typed on a machine that never disconnects
    // still goes out. Before this it waited for the next *reconnection*.
    let database = test_support::memory();
    let report = seed_small(&database, 12);
    let drafts_mailbox = report
        .mailbox(MailboxRole::Drafts)
        .expect("the seed has a Drafts folder");

    // Written before the engine exists. The engine's thread writes this same
    // database — folder discovery on link-up, then the drain — and a shared
    // in-memory SQLite answers a second writer with SQLITE_LOCKED rather than
    // waiting on it. A running Postio is file-backed and does wait; only the
    // test harness has this shape, so the test does its setup first rather
    // than racing.
    let mut identity = postio_model::Identity::new(
        report.account.id,
        postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    identity.is_default = true;
    {
        let connection = database.connection().expect("checkout");
        postio_storage::repository::IdentityRepository::new(&connection)
            .create(&mut identity)
            .expect("create the identity");
    }

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
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
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
    })
    .expect("the engine starts");

    // Wait for the link before writing anything, so what is proven is the
    // "already online" path rather than the reconnect drain.
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
        let connection = database.connection().expect("checkout");
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, Utc::now())
            .expect("save and queue");
    }

    let landed = tokio::time::timeout(std::time::Duration::from_secs(15), async {
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
    // postio-755, and the reason the first live run reported success over an
    // empty mailbox: every folder-enumerating path in the engine reads the
    // local table, and nothing ever LISTed the server to fill it. An account
    // that has never synced has no folders at all, so this is the pass
    // everything else waits on.
    let database = test_support::memory();
    let account = {
        let connection = database.connection().expect("checkout");
        let mut account = postio_model::Account::new(
            "Test",
            postio_model::EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .expect("create the account");
        account
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
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
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
    })
    .expect("the engine starts");

    let folders = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let connection = database.connection().expect("checkout");
            let found = postio_storage::repository::MailboxRepository::new(&connection)
                .list_for_account(account.id)
                .expect("list");
            drop(connection);
            if !found.is_empty() {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the engine connected and never wrote down a single folder");

    assert_eq!(folders.len(), 3, "{folders:?}");
    assert!(
        folders
            .iter()
            .any(|mailbox| mailbox.role == MailboxRole::Sent),
        "and the roles come from the server's attributes, so Sent is found \
         however the account spells it: {folders:?}"
    );
    // Nothing was asked of the engine at any point — discovery is part of
    // coming up, not something a caller has to remember.
    drop(engine);
}

#[tokio::test]
async fn a_requested_body_does_not_wait_for_the_supervisors_first_tick() {
    // #109: `postio-app::seed_the_backfill` sends a job the instant
    // `Engine::spawn` returns, so a job is reliably already queued by the
    // time an account's engine's own loop runs for the first time. Before
    // this was fixed, the very first connection attempt happened only on
    // the ticker's first fire inside that loop's `select!` -- and a job
    // that arrived first left the ticker branch to wait for a real
    // `POLL_INTERVAL` (5s), for a reason that has nothing to do with the
    // network. Fetching a body the mock server actually holds, with a job
    // sent first exactly the way production does, has to finish well under
    // that -- 2s is generous against the ~20-40ms this takes once the
    // connection is not gated behind an unrelated tick.
    let (engine, database, report, _events) = engine();
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox");
    give_the_inbox_uids(&database, inbox.id);

    // `give_the_inbox_uids` sets `uid = id` for every seeded message, and
    // `server()` only holds ten of them (UIDs 1..=10) -- so the message
    // asked for has to be one the mock can actually answer for, not merely
    // "the inbox's newest", which `seed_small` may give an id past ten.
    let message = with_store(
        &database,
        "a message the mock actually holds",
        |connection| {
            Ok(connection.query_row(
                "SELECT id FROM messages WHERE mailbox_id = ?1 AND uid BETWEEN 1 AND 10 LIMIT 1",
                [inbox.id.get()],
                |row| row.get::<_, i64>(0),
            )?)
        },
    );
    let message = postio_model::ids::MessageId::new(message);

    let wanted = engine.request_body(message).await.expect("request_body");
    assert!(
        wanted,
        "there was nothing to fetch for a message the mock holds"
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let landed = with_store(&database, "reading raw_blob_id", |connection| {
                postio_storage::repository::MessageRepository::new(connection).get(message)
            })
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
