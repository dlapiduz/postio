//! The engine against a real socket.
//!
//! Every other test in this crate drives [`MockBackend`], which is how the
//! whole engine was developed and why it can be developed at all. But a mock
//! cannot lie the way a server does: it never renumbers a mailbox mid-session,
//! never tears a connection in the middle of a body, never emits a sequence
//! number no decoder can read, and never goes quiet holding a connection open.
//!
//! So these run `postio_account::imap::ImapBackend` — real `io-imap`, real
//! session, real bytes — against `postio_account::test_server::TestServer` on an
//! ephemeral loopback port, with the same [`Quirk`] and [`Fault`] injection
//! that suite uses. Nothing here touches the network.
//!
//! The seam is still [`MailBackend`]: no protocol type appears below, and
//! everything asserted is asserted through the engine's own vocabulary.

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
use postio_storage::repository::{
    ContactRepository, MessageRepository, MessageSet, OperationQueueRepository, SyncStateRepository,
};
use postio_storage::test_support::{self, TempDatabase};
use postio_storage::{BlobStore, PooledConnection};
use postio_sync::backfill::{BackfillPolicy, BodyRequest, Outcome as BackfillOutcome, fetch_body};
use postio_sync::{
    Attention, Drainer, Outcome, Watch, WatchPolicy, Watcher, resync_mailbox, sync_mailbox,
};
use rusqlite::Connection;

const INBOX: &str = "INBOX";
const ARCHIVE: &str = "Archive";
const VALIDITY: u32 = 4_242;
const BASELINE: u64 = 900;

/// The messages both the server and the assertions are built from.
const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 23, hour, 0, 0).unwrap()
}

/// A server shaped like the account this project targets: extensions hidden
/// until after login, no special-use attributes, three messages from the
/// corpus.
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

fn local(connection: &Connection) -> (AccountId, Mailbox, Mailbox) {
    let account = test_support::account(connection);
    let inbox = test_support::mailbox(connection, &account, INBOX);
    let archive = test_support::mailbox(connection, &account, ARCHIVE);
    (account.id, inbox, archive)
}

async fn bootstrap(connection: &PooledConnection, backend: &ImapBackend, mailbox: &Mailbox) {
    sync_mailbox(connection, backend, mailbox, &CancelToken::new(), |_| {})
        .await
        .expect("bootstrap sync");
}

fn known_uids(connection: &Connection, mailbox: &Mailbox, generation: u32) -> Vec<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox.id, postio_model::Generation::new(generation))
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

// ---------------------------------------------------------------------------
// The happy path, over bytes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_engine_syncs_a_mailbox_over_a_real_socket() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    assert_eq!(report.inserted, 3);
    assert_eq!(known_uids(&connection, &inbox, VALIDITY), vec![1, 2, 3]);

    // The corpus reached the database through ENVELOPE and BODYSTRUCTURE on
    // the wire, not through a mock handing back what it was given.
    let stored = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(1),
        )
        .expect("look up")
        .expect("message 1");
    assert_eq!(stored.subject.as_deref(), Some("Tuesday walkthrough notes"));
    assert_eq!(stored.from[0].address, "ada.norwood@example.com");

    let state = SyncStateRepository::new(&connection)
        .require(inbox.id)
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    // Another client reads one message, and new mail lands.
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
        known_uids(&connection, &inbox, VALIDITY),
        vec![1, 2, 3, arrival.get()]
    );
    let seen = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(2),
        )
        .expect("look up")
        .expect("message 2");
    assert!(seen.flags.is_seen());
}

// ---------------------------------------------------------------------------
// The lies a mock cannot tell
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_uidvalidity_bump_rebuilds_rather_than_reporting_wrong_mail() {
    // The worst thing this codebase can do. The backend refuses to serve a
    // generation nobody confirmed — it reports the change once and adopts the
    // new one — and the engine has to hear that as "throw the local copy away
    // and re-enumerate", not as an error that fails the pass.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);
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
        known_uids(&connection, &inbox, VALIDITY).is_empty(),
        "every row under the stale generation must be gone"
    );
    assert_eq!(known_uids(&connection, &inbox, 9_001), vec![1, 2, 3]);
    assert_eq!(
        SyncStateRepository::new(&connection)
            .require(inbox.id)
            .expect("sync state")
            .generation,
        Some(postio_model::Generation::new(9_001))
    );
}

#[tokio::test]
async fn a_malformed_sequence_number_rebuilds_rather_than_losing_the_delta() {
    // At least one mainstream provider has shipped `* -1 FETCH (…)` under
    // QRESYNC. io-imap skips a line it cannot decode and completes the command
    // `Ok`, so the pull looks whole while a message's flags never arrived —
    // which is why postio-account counts the skips and refuses the result. The
    // engine's answer has to be a full pass: the same incremental pull would
    // lose the same line again.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);
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

    // And the change the incremental pull could not be trusted with is in
    // the database anyway, because the full pass does not use CHANGEDSINCE.
    let seen = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(2),
        )
        .expect("look up")
        .expect("message 2");
    assert!(seen.flags.is_seen());
}

/// postio-66j: a rebuild (`Coverage::Everything`) re-reads every message the
/// mailbox already holds, on purpose — that is what makes it trustworthy
/// after a pull the incremental path could not verify. Every one of those
/// re-reads must not look like a new sighting of its correspondent, or every
/// rebuild would inflate `times_seen` for mail nobody just sent.
#[tokio::test]
async fn a_rebuild_that_re_reads_known_messages_does_not_double_count_their_correspondents() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account_id, inbox, _archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let seen_before: Vec<(String, u32)> = ContactRepository::new(&connection)
        .list(Some(account_id))
        .expect("list contacts")
        .into_iter()
        .map(|contact| (contact.address.normalized(), contact.times_seen))
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
        .list(Some(account_id))
        .expect("list contacts")
        .into_iter()
        .map(|contact| (contact.address.normalized(), contact.times_seen))
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);

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

    // The fault fired once; the pool replaces the dead connection rather than
    // handing it out again, so the retry is an ordinary sync.
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);

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

// ---------------------------------------------------------------------------
// The queue, against a server that answers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_queued_flag_change_and_move_reach_a_real_server() {
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox, archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1));
    enqueue(
        &connection,
        account,
        message,
        Operation::SetFlags {
            flags: FlagSet::from_iter([Flag::Seen]),
        },
        at(9),
    );
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    );

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
    // The production order (#289): the queue row and the local move happen in
    // one transaction -- enqueue first, then the move, which nulls the row's
    // server identity as correct local-first bookkeeping. The drainer used to
    // resolve the UID from the *nulled* row, classify the move as "never
    // uploaded, so the server has nothing to change", and mark it done:
    // archived locally, never synced.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox, archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1));
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    );
    MessageRepository::new(&connection)
        .move_to(&[message], archive.id)
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
    // Delete nulls the row's server identity the same way Move does (#289),
    // so it loses its server position across the same gap.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox, trash) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(2));
    enqueue(
        &connection,
        account,
        message,
        Operation::Delete {
            from: inbox.id,
            trash: trash.id,
        },
        at(9),
    );
    MessageRepository::new(&connection)
        .move_to(&[message], trash.id)
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
async fn a_bulk_move_over_a_predicate_still_reaches_the_server() {
    // The bulk path: enqueue_set materializes one row per message before
    // move_set nulls their identities, all in one transaction (#289). The
    // snapshot each queue row carries is what the drainer has to prefer.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox, archive) = local(&connection);
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
        .expect("enqueue_set")
        .expect("the mailbox was not empty");
    MessageRepository::new(&connection)
        .move_set(&everything, archive.id)
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
    // The user-visible half of #289: with the move never landing, the next
    // resync of the source folder finds the server copy still there and
    // re-adds it -- the archived message comes back as a duplicate.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox, archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let message = message_at(&connection, &inbox, Uid::new(1));
    enqueue(
        &connection,
        account,
        message,
        Operation::Move {
            from: inbox.id,
            to: archive.id,
        },
        at(9),
    );
    MessageRepository::new(&connection)
        .move_to(&[message], archive.id)
        .expect("the local move");
    Drainer::new(&backend)
        .drain(&connection, account, at(10))
        .await
        .expect("drain");

    bootstrap(&connection, &backend, &inbox).await;

    assert_eq!(
        known_uids(&connection, &inbox, VALIDITY),
        vec![2, 3],
        "the archived message resurrected in the source folder"
    );
}

// ---------------------------------------------------------------------------
// Bodies, streamed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_backfilled_body_arrives_byte_for_byte() {
    // A real socket, a real `BODYSTRUCTURE`, and a real sectioned FETCH: the
    // text axis of ADR 0017 end to end. `attachment-pdf` is the shape the
    // whole decision is about — a few lines of words carrying a payload — and
    // what is asserted is that the words arrive intact through io-imap's own
    // parser while the payload stays on the server. A mock hands back what it
    // was given, so only a socket can prove the section numbers were right.
    let server = server().await;
    let backend = backend_for(&server).await;
    let local = on_disk();

    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("headers");

    let id = message_at(&local.connection, &local.inbox, Uid::new(2));
    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &body_request(&local.inbox, id, Uid::new(2)),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch body");

    assert!(matches!(outcome, BackfillOutcome::Stored { .. }));

    let messages = MessageRepository::new(&local.connection);
    let stored = messages.get(id).expect("get").expect("row");

    // The words came through the wire and the parser intact.
    let body = messages.body(id).expect("body").expect("the row");
    let text = body.text.expect("the message's own text");
    let fetched = text;
    let expected =
        postio_model::mime::parse(postio_model::test_corpus::load("attachment-pdf").bytes());
    assert_eq!(
        Some(fetched.trim()),
        expected.body.text.as_deref().map(str::trim),
        "the text section decoded to what the corpus says the body is"
    );

    // And the PDF did not.
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
    // The failure a mock cannot stage: the server announces an octet count
    // and then hangs up short of it. Whatever reached the sink has to be
    // discarded, and the message has to still be marked as needing its body.
    let server = server().await;
    let backend = backend_for(&server).await;
    let local = on_disk();

    sync_mailbox(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("headers");

    let id = message_at(&local.connection, &local.inbox, Uid::new(1));
    server.inject(Fault::DropConnection {
        during: "FETCH".to_owned(),
    });

    let error = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &body_request(&local.inbox, id, Uid::new(1)),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect_err("a torn body is a failure");
    assert!(is_transient(&error), "{error}");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert_eq!(
        stored.sync.body_state,
        BodyState::HeadersOnly,
        "a half-arrived body must not be recorded as a body"
    );
    assert!(stored.raw_blob_id.is_none());
}

// ---------------------------------------------------------------------------
// Watching, over a connection that really idles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_delivery_during_an_idle_wakes_the_watcher_and_the_pull_finds_it() {
    // The whole point of the watcher, end to end for the first time: a real
    // IDLE held on a real connection, a message delivered while it is
    // outstanding, and the resync that the wake-up is supposed to trigger.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    let mut watcher = Watcher::new(
        WatchPolicy {
            idle: true,
            idle_refresh: Duration::from_secs(60),
            poll_interval: Duration::from_secs(300),
        },
        &backend.capabilities().await.expect("capabilities"),
    );
    watcher.watch(inbox.id, INBOX, Attention::Push);
    watcher.watch(archive.id, ARCHIVE, Attention::Poll);

    // The first push step verifies rather than idling blind.
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
        known_uids(&connection, &inbox, VALIDITY).contains(&arrival.get()),
        "the message the wake-up was about has to be in the database"
    );
}

#[tokio::test]
async fn the_poll_floor_notices_what_no_wake_up_reported() {
    // The guarantee the watcher is shaped around: however badly push is
    // behaving — and a real server pushes nothing to a connection that was
    // not idling when the change happened — new mail surfaces within one
    // poll interval.
    let server = server().await;
    let backend = backend_for(&server).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox, _archive) = local(&connection);
    bootstrap(&connection, &backend, &inbox).await;

    // IDLE is available and on; this mailbox simply is not the one the
    // dedicated connection is holding, which is the ordinary case for every
    // folder but one.
    let mut watcher = Watcher::new(
        WatchPolicy {
            idle: true,
            idle_refresh: Duration::from_secs(60),
            poll_interval: Duration::from_secs(300),
        },
        &backend.capabilities().await.expect("capabilities"),
    );
    watcher.watch(inbox.id, INBOX, Attention::Poll);

    // A first look, to learn what the mailbox looks like when nothing has
    // happened.
    let Watch::Poll { path, .. } = watcher.next_poll(at(9)) else {
        panic!("a polled mailbox is checked");
    };
    let before = backend.status(&path).await.expect("status");
    watcher.observed(inbox.id, &before, at(9));

    // Mail lands with nobody watching for it.
    server.deliver(INBOX, TestMessage::corpus("list-thread-01-root"));

    // One poll interval later, the floor does its job.
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A file-backed database with a blob store beside it, because a blob store
/// is a directory and an in-memory database has no directory to sit next to.
struct OnDisk {
    #[allow(dead_code)]
    database: TempDatabase,
    connection: PooledConnection,
    blobs: BlobStore,
    inbox: Mailbox,
}

fn on_disk() -> OnDisk {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
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
        received_at: at(9),
        want: postio_sync::backfill::Want::Text,
    }
}

/// Whether the engine's error is one the caller is expected to retry.
fn is_transient(error: &postio_sync::SyncError) -> bool {
    match error {
        postio_sync::SyncError::Backend(error) => error.is_transient(),
        other => panic!("expected a backend failure, got {other:?}"),
    }
}

/// The local row for a message the sync just stored.
fn message_at(connection: &Connection, mailbox: &Mailbox, uid: Uid) -> MessageId {
    MessageRepository::new(connection)
        .by_uid(mailbox.id, postio_model::Generation::new(VALIDITY), uid)
        .expect("look up")
        .expect("a synced message")
        .id
}

fn enqueue(
    connection: &Connection,
    account: AccountId,
    message: MessageId,
    operation: Operation,
    when: DateTime<Utc>,
) {
    OperationQueueRepository::new(connection)
        .enqueue(account, OperationTarget::Message(message), &operation, when)
        .expect("enqueue");
}
