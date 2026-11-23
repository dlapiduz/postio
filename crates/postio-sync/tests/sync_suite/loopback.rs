use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::MailBackend;
use postio_account::cancel::CancelToken;
use postio_account::imap::{ImapBackend, PoolConfig, RustlsConnector};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{Fault, Quirk, TestMailbox, TestMessage, TestServer};
use postio_model::{
    AccountId, BodyState, Flag, FlagSet, FullResyncReason, Mailbox, MessageId, ModSeq, Operation,
    OperationTarget, Uid, UidValidity,
};
use postio_storage::Connection;
use postio_storage::repository::{
    ContactRepository, MessageRepository, MessageSet, OperationQueueRepository, SyncStateRepository,
};
use postio_storage::test_support::{self, TempStore};
use postio_storage::{BlobStore, Checkout};
use postio_sync::backfill::{BackfillPolicy, BodyRequest, Outcome as BackfillOutcome, fetch_body};
use postio_sync::{
    Attention, Drainer, Outcome, Watch, WatchPolicy, Watcher, resync_mailbox, sync_mailbox,
};

const INBOX: &str = "INBOX";
const ARCHIVE: &str = "Archive";
const VALIDITY: u32 = 4_242;
const BASELINE: u64 = 900;

const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 23, hour, 0, 0).unwrap()
}

async fn server() -> TestServer {
    TestServer::builder()
        .mailbox(
            TestMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .highest_mod_seq(ModSeq::new(BASELINE))
                .corpus(SEEDED),
        )
        .mailbox(TestMailbox::new(ARCHIVE))
        .start()
        .await
}

async fn backend_for(server: &TestServer) -> ImapBackend {
    backend_with(server, PoolConfig::default()).await
}

async fn backend_with(server: &TestServer, config: PoolConfig) -> ImapBackend {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");

    ImapBackend::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        config,
    )
}

async fn local(connection: &Connection) -> (AccountId, Mailbox, Mailbox) {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, INBOX).await;
    let archive = test_support::mailbox(connection, &account, ARCHIVE).await;
    (account.id, inbox, archive)
}

async fn bootstrap(connection: &Checkout, backend: &ImapBackend, mailbox: &Mailbox) {
    sync_mailbox(connection, backend, mailbox, &CancelToken::new(), |_| {})
        .await
        .expect("bootstrap sync");
}

async fn known_uids(connection: &Connection, mailbox: &Mailbox, generation: u32) -> Vec<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox.id, postio_model::Generation::new(generation))
        .await
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

#[tokio::test]
async fn the_engine_syncs_a_mailbox_over_a_real_socket() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    assert_eq!(report.inserted, 3);
    assert_eq!(
        known_uids(&connection, &inbox, VALIDITY).await,
        vec![1, 2, 3]
    );

    let stored = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(1),
        )
        .await
        .expect("look up")
        .expect("message 1");
    assert_eq!(stored.subject.as_deref(), Some("Tuesday walkthrough notes"));
    assert_eq!(stored.from[0].address, "ada.norwood@example.com");

    let state = SyncStateRepository::new(&connection)
        .require(inbox.id)
        .await
        .expect("sync state");
    assert_eq!(
        state.generation,
        Some(postio_model::Generation::new(VALIDITY))
    );
    assert!(state.has_synced());
}

#[tokio::test]
async fn an_incremental_resync_sees_a_flag_change_and_an_arrival() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    server.set_flags(INBOX, Uid::new(2), FlagSet::from_iter([Flag::Seen]));
    let arrival = server.deliver(INBOX, TestMessage::corpus("list-thread-01-root"));

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
            ..
        } => {
            assert_eq!(changed, 2, "the flag change and the arrival");
            assert_eq!(vanished, 0);
            assert_eq!(
                arrived.len(),
                1,
                "only the delivery is new mail, not the flag change: {arrived:?}"
            );
        }
        other => panic!("expected an incremental resync, got {other:?}"),
    }

    assert_eq!(
        known_uids(&connection, &inbox, VALIDITY).await,
        vec![1, 2, 3, arrival.get()]
    );
    let seen = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(2),
        )
        .await
        .expect("look up")
        .expect("message 2");
    assert!(seen.flags.is_seen());
}

#[tokio::test]
async fn a_uidvalidity_bump_rebuilds_rather_than_reporting_wrong_mail() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    server.set_uid_validity(INBOX, UidValidity::new(9_001));

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Full { reason, report } => {
            assert_eq!(reason, FullResyncReason::GenerationChanged);
            assert_eq!(report.inserted, 3);
        }
        other => panic!("expected a full resync, got {other:?}"),
    }

    assert!(
        known_uids(&connection, &inbox, VALIDITY).await.is_empty(),
        "every row under the stale generation must be gone"
    );
    assert_eq!(known_uids(&connection, &inbox, 9_001).await, vec![1, 2, 3]);
    assert_eq!(
        SyncStateRepository::new(&connection)
            .require(inbox.id)
            .await
            .expect("sync state")
            .generation,
        Some(postio_model::Generation::new(9_001))
    );
}

#[tokio::test]
async fn a_malformed_sequence_number_rebuilds_rather_than_losing_the_delta() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    server.set_flags(INBOX, Uid::new(2), FlagSet::from_iter([Flag::Seen]));
    server.quirk(Quirk::MalformedFetchSequenceNumber);

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Rebuilt { report } => assert!(
            report.updated >= 1,
            "a rebuild has to *re-read* what it already holds, not fill gaps: {report:?}"
        ),
        other => panic!("expected a rebuild, got {other:?}"),
    }

    let seen = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(2),
        )
        .await
        .expect("look up")
        .expect("message 2");
    assert!(seen.flags.is_seen());
}

#[tokio::test]
async fn a_rebuild_that_re_reads_known_messages_does_not_double_count_their_correspondents() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account_id, inbox, _archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let seen_before: Vec<(String, u32)> = ContactRepository::new(&connection)
        .people(10_000)
        .await
        .expect("list people")
        .into_iter()
        .flat_map(|person| person.addresses)
        .map(|owned| (owned.address.normalized(), owned.times_seen))
        .collect();
    assert!(
        !seen_before.is_empty(),
        "bootstrap must have recorded the corpus senders as contacts"
    );

    server.set_flags(INBOX, Uid::new(2), FlagSet::from_iter([Flag::Seen]));
    server.quirk(Quirk::MalformedFetchSequenceNumber);

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");
    assert!(
        matches!(outcome, Outcome::Rebuilt { .. }),
        "expected a rebuild, got {outcome:?}"
    );

    let seen_after: Vec<(String, u32)> = ContactRepository::new(&connection)
        .people(10_000)
        .await
        .expect("list people")
        .into_iter()
        .flat_map(|person| person.addresses)
        .map(|owned| (owned.address.normalized(), owned.times_seen))
        .collect();
    assert_eq!(
        seen_before, seen_after,
        "a rebuild re-reads messages already known; their correspondents' \
         counts must be exactly what bootstrap left them at"
    );
}

#[tokio::test]
async fn a_torn_fetch_fails_the_pass_and_the_next_one_succeeds() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;

    server.inject(Fault::DropConnection {
        during: "FETCH".to_owned(),
    });

    let error = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect_err("a torn connection is a failure");
    assert!(
        is_transient(&error),
        "a dropped socket has to be retryable: {error}"
    );

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the retry");
    assert_eq!(report.inserted, 3);
}

#[tokio::test]
async fn a_stalled_server_fails_the_pass_instead_of_wedging_the_engine() {
    let server = server().await;
    let backend = backend_with(
        &server,
        PoolConfig {
            command_timeout: Duration::from_millis(300),
            ..PoolConfig::default()
        },
    )
    .await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;

    server.inject(Fault::Stall {
        during: "FETCH".to_owned(),
    });

    let started = tokio::time::Instant::now();
    let error = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect_err("a server that never answers is a failure");

    assert!(is_transient(&error), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the pass gave up rather than waiting forever"
    );

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the retry");
    assert_eq!(report.inserted, 3);
}

#[tokio::test]
async fn a_queued_flag_change_and_move_reach_a_real_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1)).await;
    enqueue(
        &connection,
        account,
        message,
        Operation::SetFlags {
            flags: FlagSet::from_iter([Flag::Seen]),
        },
        at(9),
    )
    .await;
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 2, "{report:?}");
    assert!(report.failed.is_empty());
    assert_eq!(server.uids(ARCHIVE).len(), 1);
    assert_eq!(server.uids(INBOX), vec![Uid::new(2), Uid::new(3)]);
}

#[tokio::test]
async fn a_local_move_still_reaches_the_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1)).await;
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    )
    .await;
    MessageRepository::new(&connection)
        .move_to(&[message], archive.id)
        .await
        .expect("the local move");

    let report = Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(
        server.uids(ARCHIVE).len(),
        1,
        "the move never reached the server"
    );
    assert_eq!(server.uids(INBOX), vec![Uid::new(2), Uid::new(3)]);
}

#[tokio::test]
async fn a_local_delete_still_reaches_the_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, trash) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(2)).await;
    enqueue(
        &connection,
        account,
        message,
        Operation::Delete {
            from: inbox.id,
            trash: trash.id,
        },
        at(9),
    )
    .await;
    MessageRepository::new(&connection)
        .move_to(&[message], trash.id)
        .await
        .expect("the local delete");

    let report = Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(
        server.uids(ARCHIVE).len(),
        1,
        "the delete never reached the server"
    );
    assert_eq!(server.uids(INBOX), vec![Uid::new(1), Uid::new(3)]);
}

#[tokio::test]
async fn a_purge_removes_exactly_that_message_from_the_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, _trash) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(2)).await;
    enqueue(
        &connection,
        account,
        message,
        Operation::Purge { mailbox: inbox.id },
        at(9),
    )
    .await;

    let report = Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(
        server.uids(INBOX),
        vec![Uid::new(1), Uid::new(3)],
        "exactly the purged message is gone"
    );
}

#[tokio::test]
async fn a_bulk_move_over_a_predicate_still_reaches_the_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let everything = MessageSet::in_mailbox(inbox.id);
    OperationQueueRepository::new(&connection)
        .enqueue_set(
            account,
            &everything,
            &Operation::Move {
                from: inbox.id,
                to: archive.id,
            },
            at(9),
        )
        .await
        .expect("enqueue_set")
        .expect("the mailbox was not empty");
    MessageRepository::new(&connection)
        .move_set(&everything, archive.id)
        .await
        .expect("the local bulk move");

    let report = Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(
        server.uids(ARCHIVE).len(),
        3,
        "the bulk move never reached the server"
    );
    assert!(server.uids(INBOX).is_empty(), "{:?}", server.uids(INBOX));
}

#[tokio::test]
async fn a_drained_move_does_not_resurrect_on_resync() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox, archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1)).await;
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    )
    .await;
    MessageRepository::new(&connection)
        .move_to(&[message], archive.id)
        .await
        .expect("the local move");
    Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    bootstrap(&connection, &backend, &inbox).await;

    assert_eq!(
        known_uids(&connection, &inbox, VALIDITY).await,
        vec![2, 3],
        "the archived message resurrected in the source folder"
    );
}

#[tokio::test]
async fn a_backfilled_body_arrives_byte_for_byte() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let local = on_disk().await;

    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("headers");

    let id = message_at(&local.connection, &local.inbox, Uid::new(2)).await;
    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &body_request(&local.inbox, id, Uid::new(2)),
        BackfillPolicy::default().max_inline_bytes,
        None,
        &CancelToken::new(),
    )
    .await
    .expect("fetch body");

    assert!(matches!(outcome, BackfillOutcome::Stored { .. }));

    let messages = MessageRepository::new(&local.connection);
    let stored = messages.get(id).await.expect("get").expect("row");

    let body = messages.body(id).await.expect("body").expect("the row");
    let text = body.text.expect("the message's own text");
    let fetched = text;
    let expected =
        postio_model::mime::parse(postio_model::test_corpus::load("attachment-pdf").bytes());
    assert_eq!(
        Some(fetched.trim()),
        expected.body.text.as_deref().map(str::trim),
        "the text section decoded to what the corpus says the body is"
    );

    assert!(
        stored.raw_blob_id.is_none(),
        "no whole-message fetch happened, so there is no raw blob to keep"
    );
    assert_eq!(
        stored.sync.body_state,
        BodyState::Partial,
        "text local, payload still on the server"
    );
    assert!(
        stored.attachments.iter().all(|part| part.blob_id.is_none()),
        "nothing pulled the attachment's bytes"
    );
    let attachment_bytes: u64 = stored.attachments.iter().map(|part| part.size).sum();
    assert!(
        matches!(outcome, BackfillOutcome::Stored { bytes } if bytes < attachment_bytes),
        "less crossed the wire than the payload alone would have cost"
    );
}

#[tokio::test]
async fn a_body_torn_off_the_socket_stores_nothing() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let local = on_disk().await;

    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("headers");

    let id = message_at(&local.connection, &local.inbox, Uid::new(1)).await;
    server.inject(Fault::DropConnection {
        during: "FETCH".to_owned(),
    });

    let error = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &body_request(&local.inbox, id, Uid::new(1)),
        BackfillPolicy::default().max_inline_bytes,
        None,
        &CancelToken::new(),
    )
    .await
    .expect_err("a torn body is a failure");
    assert!(is_transient(&error), "{error}");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(
        stored.sync.body_state,
        BodyState::HeadersOnly,
        "a half-arrived body must not be recorded as a body"
    );
    assert!(stored.raw_blob_id.is_none());
}

#[tokio::test]
async fn a_delivery_during_an_idle_wakes_the_watcher_and_the_pull_finds_it() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let mut watcher = Watcher::new(
        WatchPolicy {
            idle: true,
            idle_refresh: Duration::from_secs(60),
            poll_interval: Duration::from_secs(300),
            manual: false,
        },
        &backend.capabilities().await.expect("capabilities"),
    );
    watcher.watch(inbox.id, INBOX, Attention::Push);
    watcher.watch(archive.id, ARCHIVE, Attention::Poll);

    let Watch::Poll { .. } = watcher.next_push(at(9)) else {
        panic!("the first push step is a verifying STATUS");
    };
    let status = backend.status(INBOX).await.expect("status");
    watcher.observed(inbox.id, &status, at(9));

    let Watch::Idle {
        path,
        timeout,
        cancel,
        ..
    } = watcher.next_push(at(9))
    else {
        panic!("a verified mailbox is idled on");
    };
    assert_eq!(path, INBOX);

    let (events, arrival) = tokio::join!(backend.idle(&path, timeout, &cancel), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        server.deliver(INBOX, TestMessage::corpus("list-thread-01-root"))
    });
    let events = events.expect("idle");

    assert!(
        !events.is_empty(),
        "the server announced the delivery, so IDLE must not report silence"
    );
    assert!(
        watcher.woke(inbox.id, &events, at(10)).needs_resync(),
        "an announced change needs a pull"
    );

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");
    assert!(
        known_uids(&connection, &inbox, VALIDITY)
            .await
            .contains(&arrival.get()),
        "the message the wake-up was about has to be in the database"
    );
}

#[tokio::test]
async fn the_poll_floor_notices_what_no_wake_up_reported() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox, _archive) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let mut watcher = Watcher::new(
        WatchPolicy {
            idle: true,
            idle_refresh: Duration::from_secs(60),
            poll_interval: Duration::from_secs(300),
            manual: false,
        },
        &backend.capabilities().await.expect("capabilities"),
    );
    watcher.watch(inbox.id, INBOX, Attention::Poll);

    let Watch::Poll { path, .. } = watcher.next_poll(at(9)) else {
        panic!("a polled mailbox is checked");
    };
    let before = backend.status(&path).await.expect("status");
    watcher.observed(inbox.id, &before, at(9));

    server.deliver(INBOX, TestMessage::corpus("list-thread-01-root"));

    let Watch::Poll { path, .. } = watcher.next_poll(at(9) + chrono::TimeDelta::seconds(300))
    else {
        panic!("the interval elapsed, so the mailbox is due");
    };
    let after = backend.status(&path).await.expect("status");

    assert!(
        watcher
            .observed(inbox.id, &after, at(9) + chrono::TimeDelta::seconds(300))
            .needs_resync(),
        "a mailbox that changed under a quiet connection still has to be pulled"
    );
}

struct OnDisk {
    #[allow(dead_code)]
    database: TempStore,
    connection: Checkout,
    blobs: BlobStore,
    inbox: Mailbox,
}

async fn on_disk() -> OnDisk {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    OnDisk {
        database,
        connection,
        blobs,
        inbox,
    }
}

fn body_request(mailbox: &Mailbox, message: MessageId, uid: Uid) -> BodyRequest {
    BodyRequest {
        message,
        mailbox: mailbox.id,
        path: mailbox.path.clone(),
        remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:{uid}")),
        uid,
        size: 0,
        text_size: 0,
        rank: postio_sync::order::sync_priority(mailbox.role),
        received_at: at(9),
        want: postio_sync::backfill::Want::Text,
    }
}

fn is_transient(error: &postio_sync::SyncError) -> bool {
    match error {
        postio_sync::SyncError::Backend(error) => error.is_transient(),
        other => panic!("expected a backend failure, got {other:?}"),
    }
}

async fn message_at(connection: &Connection, mailbox: &Mailbox, uid: Uid) -> MessageId {
    MessageRepository::new(connection)
        .by_uid(mailbox.id, postio_model::Generation::new(VALIDITY), uid)
        .await
        .expect("look up")
        .expect("a synced message")
        .id
}

async fn enqueue(
    connection: &Connection,
    account: AccountId,
    message: MessageId,
    operation: Operation,
    when: DateTime<Utc>,
) {
    OperationQueueRepository::new(connection)
        .enqueue(account, OperationTarget::Message(message), &operation, when)
        .await
        .expect("enqueue");
}
