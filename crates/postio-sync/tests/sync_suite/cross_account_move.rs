use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
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

const RAW_ANONYMOUS: &[u8] =
    b"From: Ada Lovelace <ada@example.com>\r\nSubject: Analytical engine\r\n\r\nNotes.\r\n";

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 2, hour, 0, 0).unwrap()
}

struct World {
    database: test_support::TempStore,
    blobs: BlobStore,
    source_account: AccountId,
    target_account: AccountId,
    source_inbox: MailboxId,
    target_inbox: MailboxId,
    source_message: MessageId,
    target_message: MessageId,
    saga: postio_model::ids::CrossAccountMoveId,
}

async fn world(raw: &[u8], with_message_id: bool) -> World {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("blobs");
    let connection = database.connect().await.expect("checkout");

    let (source, source_inbox) = test_support::account_with_inbox(&connection).await;
    let mut target = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut target)
        .await
        .expect("target account");
    let target_inbox = test_support::mailbox(&connection, &target, "INBOX")
        .await
        .id;

    let mut message = Message::new(source.id, source_inbox, at(8));
    message.server.uid = Some(Uid::new(1));
    message.server.uid_validity = Some(UidValidity::new(1));
    message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
    if with_message_id {
        message.rfc_message_id = Some(RfcMessageId::new("<engine@example.com>"));
    }
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("source message");

    // The provisional copy `relocate_rows` makes for every cross-account move; born with no server
    // identity (#940).
    let mut copy = message.clone();
    copy.id = MessageId::UNASSIGNED;
    copy.account_id = target.id;
    copy.mailbox_id = target_inbox;
    copy.thread_id = None;
    copy.server = postio_model::ServerIdentifiers::default();
    let target_message = MessageRepository::new(&connection)
        .create(&mut copy)
        .await
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
        .await
        .expect("saga");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            target.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountCopy { saga },
            at(9),
        )
        .await
        .expect("enqueue copy");
    queue
        .enqueue(
            source.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountRemove { saga },
            at(9),
        )
        .await
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

async fn drain_at(world: &World, backend: &MockBackend, account: AccountId, hour: u32) {
    let connection = world.database.connect().await.expect("checkout");
    Drainer::new(backend)
        .with_blobs(&world.blobs)
        .drain(&connection, account, at(hour))
        .await
        .expect("a drain pass");
}

async fn phase(world: &World) -> MovePhase {
    let connection = world.database.connect().await.expect("checkout");
    CrossAccountMoveRepository::new(&connection)
        .get(world.saga)
        .await
        .expect("read")
        .expect("the saga")
        .phase
}

async fn messages_in(backend: &MockBackend, mailbox: &str) -> usize {
    backend.status(mailbox).await.expect("status").exists as usize
}

#[tokio::test]
async fn the_move_completes_and_the_only_order_is_copy_confirm_remove() {
    let world = world(RAW, true).await;
    let source = source_server().await;
    let target = target_server(true).await;

    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "nothing may be deleted before the copy is confirmed"
    );
    assert_eq!(phase(&world).await, MovePhase::Copying);

    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);
    assert_eq!(phase(&world).await, MovePhase::Confirmed);

    drain_at(&world, &source, world.source_account, 20).await;
    assert_eq!(messages_in(&source, "INBOX").await, 0);
    assert_eq!(phase(&world).await, MovePhase::Done);
}

/// #531: the confirmation writes the destination identity onto the row the user sees, or the next
/// target sync would create a second row and an inverse saga would have no coordinate.
#[tokio::test]
async fn confirming_writes_the_identity_onto_the_row_the_user_sees() {
    let world = world(RAW, true).await;
    let copy = world.target_message;
    let target = target_server(true).await;

    let before = {
        let connection = world.database.connect().await.expect("checkout");
        MessageRepository::new(&connection)
            .get(copy)
            .await
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
    assert_eq!(phase(&world).await, MovePhase::Confirmed);

    let connection = world.database.connect().await.expect("checkout");
    let saga = CrossAccountMoveRepository::new(&connection)
        .get(world.saga)
        .await
        .expect("read")
        .expect("the saga");
    let confirmed = saga
        .confirmed_remote_id
        .expect("the saga records what the append proved");
    let stored = MessageRepository::new(&connection)
        .get(copy)
        .await
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
    let world = world(RAW, true).await;
    let target = target_server(true).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);

    {
        let connection = world.database.connect().await.expect("checkout");
        connection
            .execute(
                "UPDATE operation_queue SET state = 'pending' WHERE account_id = ?1",
                [world.target_account.get()],
            )
            .await
            .expect("reset the row");
    }
    drain(&world, &target, world.target_account).await;
    assert_eq!(
        messages_in(&target, "INBOX").await,
        1,
        "the replayed APPEND must find the earlier copy, not add one"
    );
    assert_eq!(phase(&world).await, MovePhase::Confirmed);
}

#[tokio::test]
async fn without_uidplus_the_search_confirms_instead() {
    let world = world(RAW, true).await;
    let target = target_server(false).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(messages_in(&target, "INBOX").await, 1);
    assert_eq!(
        phase(&world).await,
        MovePhase::Confirmed,
        "no UIDPLUS is a slower path, not a blocker: the Message-ID search \
         is the proof"
    );
}

#[tokio::test]
async fn unconfirmable_stops_at_phase_two_and_deletes_nothing() {
    let world = world(RAW_ANONYMOUS, false).await;
    let source = source_server().await;
    let target = target_server(false).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world).await, MovePhase::Unconfirmed);

    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "unconfirmed means the source copy stays, however long it takes"
    );
}

/// #531 / ADR 0026: phase 3 uses the identity its own queue row snapshotted, not the live row,
/// which an inverse saga's provisional copy leaves empty (#940).
/// The fixture nulls the row's identity after enqueueing.
#[tokio::test]
async fn the_removal_uses_the_coordinate_its_queue_row_snapshotted() {
    let world = world(RAW, true).await;
    let source = source_server().await;
    let target = target_server(true).await;

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world).await, MovePhase::Confirmed);

    {
        let connection = world.database.connect().await.expect("checkout");
        connection
            .execute(
                "UPDATE messages SET remote_id = NULL, uid = NULL WHERE id = ?1",
                [world.source_message.get()],
            )
            .await
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
    assert_eq!(phase(&world).await, MovePhase::Done);
}

#[tokio::test]
async fn a_vanished_destination_aborts_with_the_source_intact() {
    let world = world(RAW, true).await;
    let source = source_server().await;
    let target = target_server(true).await;
    {
        let connection = world.database.connect().await.expect("checkout");
        postio_storage::repository::MailboxRepository::new(&connection)
            .delete(world.target_inbox)
            .await
            .expect("the destination goes away");
    }

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world).await, MovePhase::Aborted);
    assert_eq!(messages_in(&target, "INBOX").await, 0);

    drain(&world, &source, world.source_account).await;
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "an aborted move leaves the message exactly where it was (Q13)"
    );
    let connection = world.database.connect().await.expect("checkout");
    assert!(
        MessageRepository::new(&connection)
            .get(world.source_message)
            .await
            .expect("read")
            .is_some(),
        "the local source row survived the abort"
    );
    let _ = world.source_inbox;
}

/// #531: the inverse saga's removal reaches the target server with the target's coordinates; a
/// wrong one would expunge nothing and settle `done`.
/// Driven the whole way so every coordinate is one a server issued.
#[tokio::test]
async fn the_inverse_removal_reaches_the_target_server_with_its_own_coordinates() {
    let world = world(RAW, true).await;
    let source = source_server().await;
    let target = target_server(true).await;

    drain(&world, &target, world.target_account).await;
    drain(&world, &source, world.source_account).await;
    assert_eq!(phase(&world).await, MovePhase::Done);
    assert_eq!(messages_in(&source, "INBOX").await, 0, "A gave it up");
    assert_eq!(messages_in(&target, "INBOX").await, 1, "B has it");

    let copy_identity = {
        let connection = world.database.connect().await.expect("checkout");
        MessageRepository::new(&connection)
            .get(world.target_message)
            .await
            .expect("read")
            .expect("the copy")
            .server
            .remote_id
            .expect("phase 2 wrote what the append proved")
    };

    let forward_blob = {
        let connection = world.database.connect().await.expect("checkout");
        CrossAccountMoveRepository::new(&connection)
            .get(world.saga)
            .await
            .expect("read")
            .expect("the forward saga")
            .raw_blob_id
    };
    let inverse = {
        let connection = world.database.connect().await.expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let inverse = sagas
            .create(&NewCrossAccountMove {
                source_message: world.target_message,
                source_account: world.target_account,
                source_mailbox: world.target_inbox,
                target_account: world.source_account,
                target_mailbox: world.source_inbox,
                target_message: Some(world.source_message),
                raw_blob_id: forward_blob.clone(),
                rfc_message_id: Some("<engine@example.com>".to_owned()),
            })
            .await
            .expect("the inverse saga");
        let queue = OperationQueueRepository::new(&connection);
        queue
            .enqueue(
                world.source_account,
                OperationTarget::Message(world.source_message),
                &Operation::CrossAccountCopy { saga: inverse },
                at(11),
            )
            .await
            .expect("enqueue the copy back");
        queue
            .enqueue(
                world.target_account,
                OperationTarget::Message(world.target_message),
                &Operation::CrossAccountRemove { saga: inverse },
                at(11),
            )
            .await
            .expect("enqueue the removal from B");
        MessageRepository::new(&connection)
            .set_deleted_locally(&[world.source_message], false)
            .await
            .expect("the original comes back");
        inverse
    };

    drain_at(&world, &source, world.source_account, 12).await;
    let phase_now = {
        let connection = world.database.connect().await.expect("checkout");
        CrossAccountMoveRepository::new(&connection)
            .get(inverse)
            .await
            .expect("read")
            .expect("the inverse saga")
            .phase
    };
    assert_eq!(
        phase_now,
        MovePhase::Confirmed,
        "the inverse could not put the message back on account A, so its \
         removal from B must not run -- which is the confirm-before-delete \
         rule doing its job, and this test failing for the right reason"
    );

    drain_at(&world, &target, world.target_account, 13).await;

    assert_eq!(
        messages_in(&target, "INBOX").await,
        0,
        "the inverse removal settled without expunging anything from the \
         target server: it reached nothing and reported success, and account \
         B keeps a copy of a message the user was told came back"
    );
    assert_eq!(
        messages_in(&source, "INBOX").await,
        1,
        "and the message is back on the server it came from, exactly once"
    );

    // The count is the coordinate assertion: a foreign `RemoteId` fails `wire_uid`'s generation
    // check or names a UID B never issued, leaving the message in place.
    let _ = copy_identity;
}

#[tokio::test]
async fn a_copy_replayed_after_the_saga_already_aborted_is_obsolete() {
    let world = world(RAW, true).await;
    let target = target_server(true).await;
    {
        let connection = world.database.connect().await.expect("checkout");
        postio_storage::repository::MailboxRepository::new(&connection)
            .delete(world.target_inbox)
            .await
            .expect("the destination goes away");
    }

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world).await, MovePhase::Aborted);

    {
        let connection = world.database.connect().await.expect("checkout");
        connection
            .execute(
                "UPDATE operation_queue SET state = 'pending' WHERE account_id = ?1",
                [world.target_account.get()],
            )
            .await
            .expect("re-queue the copy as a restart would find it");
    }
    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world).await,
        MovePhase::Aborted,
        "a replayed copy against an already-aborted saga must not move the \
         phase again"
    );
    assert_eq!(
        messages_in(&target, "INBOX").await,
        0,
        "and must not try the destination a second time either"
    );
}

#[tokio::test]
async fn a_search_failure_while_confirming_backs_off_rather_than_failing() {
    let world = world(RAW, true).await;
    let target = target_server(false).await;
    target.inject_after(
        1,
        Fault::Rejected("SEARCH not permitted just now".to_owned()),
    );

    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world).await,
        MovePhase::Copying,
        "a search failure must leave the saga exactly where it was, not \
         abort it or park it unconfirmed"
    );
    assert_eq!(
        messages_in(&target, "INBOX").await,
        0,
        "and nothing should have been uploaded on the strength of a search \
         that never answered"
    );
}

#[tokio::test]
async fn an_upload_failure_backs_off_rather_than_failing() {
    let world = world(RAW_ANONYMOUS, false).await;
    let target = target_server(false).await;
    target.inject_after(1, Fault::Rejected("APPEND refused".to_owned()));

    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world).await,
        MovePhase::Copying,
        "an upload failure must leave the saga exactly where it was"
    );
    assert_eq!(messages_in(&target, "INBOX").await, 0);
}

#[tokio::test]
async fn a_removal_with_no_coordinate_anywhere_settles_without_reaching_a_server() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("blobs");
    let connection = database.connect().await.expect("checkout");

    let (source, source_inbox) = test_support::account_with_inbox(&connection).await;
    let mut target = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut target)
        .await
        .expect("target account");
    let target_inbox = test_support::mailbox(&connection, &target, "INBOX")
        .await
        .id;

    let mut message = Message::new(source.id, source_inbox, at(8));
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("source message");

    let saga = CrossAccountMoveRepository::new(&connection)
        .create(&NewCrossAccountMove {
            source_message,
            source_account: source.id,
            source_mailbox: source_inbox,
            target_account: target.id,
            target_mailbox: target_inbox,
            target_message: None,
            raw_blob_id: None,
            rfc_message_id: None,
        })
        .await
        .expect("saga");
    CrossAccountMoveRepository::new(&connection)
        .transition(saga, MovePhase::Confirmed)
        .await
        .expect("confirmed");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            source.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountRemove { saga },
            at(9),
        )
        .await
        .expect("enqueue remove");
    drop(connection);

    let source_backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").uid_validity(UidValidity::new(1)))
        .build();
    source_backend.connect().await.expect("connect");

    let connection = database.connect().await.expect("checkout");
    Drainer::new(&source_backend)
        .with_blobs(&blobs)
        .drain(&connection, source.id, at(10))
        .await
        .expect("a drain pass");

    let phase = CrossAccountMoveRepository::new(&connection)
        .get(saga)
        .await
        .expect("read")
        .expect("the saga")
        .phase;
    assert_eq!(
        phase,
        MovePhase::Done,
        "a removal with no coordinate anywhere must still settle -- \
         retrying can never produce a coordinate that does not exist"
    );
}

#[tokio::test]
async fn a_copy_with_no_local_blob_yet_backs_off_rather_than_uploading_nothing() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("blobs");
    let connection = database.connect().await.expect("checkout");

    let (source, source_inbox) = test_support::account_with_inbox(&connection).await;
    let mut target = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut target)
        .await
        .expect("target account");
    let target_inbox = test_support::mailbox(&connection, &target, "INBOX")
        .await
        .id;

    let mut message = Message::new(source.id, source_inbox, at(8));
    message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("source message");
    let mut copy = message.clone();
    copy.id = MessageId::UNASSIGNED;
    copy.account_id = target.id;
    copy.mailbox_id = target_inbox;
    copy.server = postio_model::ServerIdentifiers::default();
    let target_message = MessageRepository::new(&connection)
        .create(&mut copy)
        .await
        .expect("the provisional copy");

    let saga = CrossAccountMoveRepository::new(&connection)
        .create(&NewCrossAccountMove {
            source_message,
            source_account: source.id,
            source_mailbox: source_inbox,
            target_account: target.id,
            target_mailbox: target_inbox,
            target_message: Some(target_message),
            raw_blob_id: None,
            rfc_message_id: None,
        })
        .await
        .expect("saga");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            target.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountCopy { saga },
            at(9),
        )
        .await
        .expect("enqueue copy");
    drop(connection);

    let target_backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    target_backend.connect().await.expect("connect");

    let connection = database.connect().await.expect("checkout");
    Drainer::new(&target_backend)
        .with_blobs(&blobs)
        .drain(&connection, target.id, at(10))
        .await
        .expect("a drain pass");

    let phase = CrossAccountMoveRepository::new(&connection)
        .get(saga)
        .await
        .expect("read")
        .expect("the saga")
        .phase;
    assert_eq!(
        phase,
        MovePhase::Copying,
        "a missing local blob must leave the saga exactly where it was"
    );
    assert_eq!(
        target_backend.status("INBOX").await.expect("status").exists,
        0,
        "and nothing should have been appended with no bytes to send"
    );
}
