//! The cross-account move saga, drained (#188, ADR 0005 Q9).
//!
//! Each design constraint the ADR states is a test here, against two
//! `MockBackend`s standing in for two servers and one store holding both
//! accounts. What the matrix is really about: **however the two drainers
//! interleave, replay, or die, the source copy outlives anything short of a
//! confirmed copy in the target.**

use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_model::ids::{AccountId, MailboxId, MessageId, Uid, UidValidity};
use postio_model::{Message, Operation, OperationTarget, RfcMessageId};
use postio_storage::repository::{
    CrossAccountMoveRepository, MessageRepository, MovePhase, NewCrossAccountMove,
    OperationQueueRepository,
};
use postio_storage::{BlobStore, test_support};
use postio_sync::drain::Drainer;

const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
    Message-ID: <engine@example.com>\r\n\
    Subject: Analytical engine\r\n\r\nNotes.\r\n";

/// The same message, with no Message-ID to confirm by.
const RAW_ANONYMOUS: &[u8] =
    b"From: Ada Lovelace <ada@example.com>\r\nSubject: Analytical engine\r\n\r\nNotes.\r\n";

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 2, hour, 0, 0).unwrap()
}

/// Two accounts in one store, a saga between them, and both queue halves.
struct World {
    database: test_support::TempDatabase,
    blobs: BlobStore,
    source_account: AccountId,
    target_account: AccountId,
    source_inbox: MailboxId,
    target_inbox: MailboxId,
    source_message: MessageId,
    /// The provisional copy in the target account — the row the user sees
    /// there at once, and the one phase 2 has to give an identity to.
    target_message: MessageId,
    saga: postio_model::ids::CrossAccountMoveId,
}

fn world(raw: &[u8], with_message_id: bool) -> World {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("blobs");
    let connection = database.connection().expect("checkout");

    let (source, source_inbox) = test_support::account_with_inbox(&connection);
    let mut target = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut target)
        .expect("target account");
    let target_inbox = test_support::mailbox(&connection, &target, "INBOX").id;

    let mut message = Message::new(source.id, source_inbox, at(8));
    message.server.uid = Some(Uid::new(1));
    message.server.uid_validity = Some(UidValidity::new(1));
    message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
    if with_message_id {
        message.rfc_message_id = Some(RfcMessageId::new("<engine@example.com>"));
    }
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("source message");

    // The provisional copy the user sees in the target the instant they
    // press the key — `relocate_rows` makes one for every cross-account
    // move, so a fixture without one is not the shape this code runs
    // against. Born with no server identity at all (#940): account B's
    // server has never seen it.
    let mut copy = message.clone();
    copy.id = MessageId::UNASSIGNED;
    copy.account_id = target.id;
    copy.mailbox_id = target_inbox;
    copy.thread_id = None;
    copy.server = postio_model::ServerIdentifiers::default();
    let target_message = MessageRepository::new(&connection)
        .create(&mut copy)
        .expect("the provisional copy");

    let blob = blobs.put(raw).expect("raw blob");
    let saga = CrossAccountMoveRepository::new(&connection)
        .create(&NewCrossAccountMove {
            source_message,
            source_account: source.id,
            source_mailbox: source_inbox,
            target_account: target.id,
            target_mailbox: target_inbox,
            target_message: Some(target_message),
            raw_blob_id: Some(blob.as_str().to_owned()),
            rfc_message_id: with_message_id.then(|| "<engine@example.com>".to_owned()),
        })
        .expect("saga");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            target.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountCopy { saga },
            at(9),
        )
        .expect("enqueue copy");
    queue
        .enqueue(
            source.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountRemove { saga },
            at(9),
        )
        .expect("enqueue remove");
    drop(connection);

    World {
        database,
        blobs,
        source_account: source.id,
        target_account: target.id,
        source_inbox,
        target_inbox,
        source_message,
        target_message,
        saga,
    }
}

/// The source server: the message at UID 1 in its inbox.
async fn source_server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new("INBOX")
                .uid_validity(UidValidity::new(1))
                .message(MockMessage::new(RAW.to_vec())),
        )
        .build();
    backend.connect().await.expect("connect");
    backend
}

/// The target server: an empty inbox. `uid_plus` decides whether APPEND
/// proves anything.
async fn target_server(uid_plus: bool) -> MockBackend {
    let mut builder = MockBackend::builder().mailbox(MockMailbox::new("INBOX"));
    builder = if uid_plus {
        builder.capabilities(["IMAP4rev1", "UIDPLUS"])
    } else {
        builder.capabilities(["IMAP4rev1"])
    };
    let backend = builder.build();
    backend.connect().await.expect("connect");
    backend
}

async fn drain(world: &World, backend: &MockBackend, account: AccountId) {
    drain_at(world, backend, account, 10).await;
}

/// A refused step backs off, so a later pass has to come visibly later.
async fn drain_at(world: &World, backend: &MockBackend, account: AccountId, hour: u32) {
    let connection = world.database.connection().expect("checkout");
    Drainer::new(backend)
        .with_blobs(&world.blobs)
        .drain(&connection, account, at(hour))
        .await
        .expect("a drain pass");
}

fn phase(world: &World) -> MovePhase {
    let connection = world.database.connection().expect("checkout");
    CrossAccountMoveRepository::new(&connection)
        .get(world.saga)
        .expect("read")
        .expect("the saga")
        .phase
}

async fn messages_in(backend: &MockBackend, mailbox: &str) -> usize {
    backend.status(mailbox).await.expect("status").exists as usize
}

#[tokio::test]
async fn the_move_completes_and_the_only_order_is_copy_confirm_remove() {
    let world = world(RAW, true);
    let source = source_server().await;
    let target = target_server(true).await;

    // The remove drained first — the interleaving that must refuse. The
    // source server still holds the message afterwards, whatever the queue
    // wanted.
    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "nothing may be deleted before the copy is confirmed"
    );
    assert_eq!(phase(&world), MovePhase::Copying);

    // The copy runs: APPEND, confirmed by APPENDUID.
    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);
    assert_eq!(phase(&world), MovePhase::Confirmed);

    // Now — and only now — the remove goes through. Hours later, because
    // the refused attempt backed off like any deferred operation.
    drain_at(&world, &source, world.source_account, 20).await;
    assert_eq!(messages_in(&source, "INBOX").await, 0);
    assert_eq!(phase(&world), MovePhase::Done);
}

/// #531, and the forward path's own reconciliation.
///
/// The confirmation proves where the message landed on the target server —
/// `APPENDUID` carries the destination's `UIDVALIDITY` as well as its UID,
/// so the identity is whole (ADR 0026). Writing it only onto the saga leaves
/// the row the user is looking at claiming no server identity, with two
/// consequences: the target account's next sync matches a fetched message by
/// `find_by_remote_id(mailbox, remote_id)` and so produces a *second* row for
/// the same message, and an inverse saga has no coordinate to name the copy
/// it must remove.
#[tokio::test]
async fn confirming_writes_the_identity_onto_the_row_the_user_sees() {
    let world = world(RAW, true);
    let copy = world.target_message;
    let target = target_server(true).await;

    let before = {
        let connection = world.database.connection().expect("checkout");
        MessageRepository::new(&connection)
            .get(copy)
            .expect("read")
            .expect("the copy")
            .server
    };
    assert_eq!(
        before,
        postio_model::ServerIdentifiers::default(),
        "the provisional copy starts with no server identity, or this test \
         cannot show that the confirmation is what gives it one"
    );

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world), MovePhase::Confirmed);

    let connection = world.database.connection().expect("checkout");
    let saga = CrossAccountMoveRepository::new(&connection)
        .get(world.saga)
        .expect("read")
        .expect("the saga");
    let confirmed = saga
        .confirmed_remote_id
        .expect("the saga records what the append proved");
    let stored = MessageRepository::new(&connection)
        .get(copy)
        .expect("read")
        .expect("the copy")
        .server;

    assert_eq!(
        stored.remote_id.as_ref(),
        Some(&confirmed),
        "the saga knows where the message landed and the row does not, so \
         the target account's next sync will not recognise its own copy"
    );
    assert!(
        stored.is_known_to_server(),
        "the target server has seen this message now, and the row still says \
         otherwise"
    );
}

#[tokio::test]
async fn a_replayed_copy_finds_the_first_copy_and_makes_no_second() {
    // A crash after the APPEND but before the queue row settles replays the
    // operation. Idempotency is by Message-ID: the re-run confirms the copy
    // that is already there.
    let world = world(RAW, true);
    let target = target_server(true).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);

    // The crash: the settled row is put back to pending, as a restart that
    // lost the settle would leave it.
    {
        let connection = world.database.connection().expect("checkout");
        connection
            .execute(
                "UPDATE operation_queue SET state = 'pending' WHERE account_id = ?1",
                [world.target_account.get()],
            )
            .expect("reset the row");
    }
    drain(&world, &target, world.target_account).await;
    assert_eq!(
        messages_in(&target, "INBOX").await,
        1,
        "the replayed APPEND must find the earlier copy, not add one"
    );
    assert_eq!(phase(&world), MovePhase::Confirmed);
}

#[tokio::test]
async fn without_uidplus_the_search_confirms_instead() {
    let world = world(RAW, true);
    let target = target_server(false).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);
    assert_eq!(
        phase(&world),
        MovePhase::Confirmed,
        "no UIDPLUS is a slower path, not a blocker: the Message-ID search \
         is the proof"
    );
}

#[tokio::test]
async fn unconfirmable_stops_at_phase_two_and_deletes_nothing() {
    // No UIDPLUS and no Message-ID: the append lands but nothing can prove
    // it. The saga parks in `unconfirmed`, the operation fails loudly, and
    // the remove keeps refusing — the ADR's "stop and ask", exactly.
    let world = world(RAW_ANONYMOUS, false);
    let source = source_server().await;
    let target = target_server(false).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world), MovePhase::Unconfirmed);

    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "unconfirmed means the source copy stays, however long it takes"
    );
}

/// #531 / ADR 0026: phase 3 uses the identity its own queue row snapshotted.
///
/// `remove` re-derives the source coordinate from the live message row, and
/// reads "no `remote_id` on the row" as "the server copy is already gone" —
/// settling the saga `done` without touching a server. That is right for a
/// row whose copy really did vanish and wrong for a row that never had an
/// identity, which is precisely what an inverse saga hands it: the
/// provisional copy in the target account is born with none (#940).
///
/// The mechanism to use instead already exists. `operation_queue.
/// source_remote_id` is filled by `enqueue` for every message-targeted
/// operation, reading the row *before* the caller's local write can null it
/// — the ordering #289 exists to enforce — and `Move` and `Delete` already
/// prefer it for exactly this reason.
///
/// The fixture nulls the row's identity after enqueueing, which is the state
/// an inverse saga produces and the one this code has never been run
/// against.
#[tokio::test]
async fn the_removal_uses_the_coordinate_its_queue_row_snapshotted() {
    let world = world(RAW, true);
    let source = source_server().await;
    let target = target_server(true).await;

    // Phase 1-2 on the target, so the saga reaches `confirmed`.
    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world), MovePhase::Confirmed);

    // The live row loses its identity, the queue row keeps its snapshot.
    {
        let connection = world.database.connection().expect("checkout");
        connection
            .execute(
                "UPDATE messages SET remote_id = NULL, uid = NULL WHERE id = ?1",
                [world.source_message.get()],
            )
            .expect("clear the live coordinates");
    }
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "the source server still holds the message, or the assertion below \
         cannot fail"
    );

    drain(&world, &source, world.source_account).await;

    assert_eq!(
        messages_in(&source, "INBOX").await,
        0,
        "phase 3 settled without expunging anything: it read the live row, \
         found no coordinate, and called that 'already gone'. The source \
         copy is still on the server and the move reports success — which is \
         a duplicate the user did not ask for, and the same silence an \
         inverse saga would hit every time"
    );
    assert_eq!(phase(&world), MovePhase::Done);
}

#[tokio::test]
async fn a_vanished_destination_aborts_with_the_source_intact() {
    // Q13: the target folder was deleted (or its account removed) while the
    // saga was in flight. The saga aborts; the source copy is untouched.
    let world = world(RAW, true);
    let source = source_server().await;
    let target = target_server(true).await;
    {
        let connection = world.database.connection().expect("checkout");
        postio_storage::repository::MailboxRepository::new(&connection)
            .delete(world.target_inbox)
            .expect("the destination goes away");
    }

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world), MovePhase::Aborted);
    assert_eq!(messages_in(&target, "INBOX").await, 0);

    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "an aborted move leaves the message exactly where it was (Q13)"
    );
    // And locally too: the source row is still there to be shown again.
    let connection = world.database.connection().expect("checkout");
    assert!(
        MessageRepository::new(&connection)
            .get(world.source_message)
            .expect("read")
            .is_some(),
        "the local source row survived the abort"
    );
    let _ = world.source_inbox;
}
