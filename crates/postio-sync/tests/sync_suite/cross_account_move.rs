//! The cross-account move saga, drained (#188, ADR 0005 Q9).
//!
//! Each design constraint the ADR states is a test here, against two
//! `MockBackend`s standing in for two servers and one store holding both
//! accounts. What the matrix is really about: **however the two drainers
//! interleave, replay, or die, the source copy outlives anything short of a
//! confirmed copy in the target.**

use chrono::{DateTime, TimeZone, Utc};
use postio_imap::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
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

    let blob = blobs.put(raw).expect("raw blob");
    let saga = CrossAccountMoveRepository::new(&connection)
        .create(&NewCrossAccountMove {
            source_message,
            source_account: source.id,
            source_mailbox: source_inbox,
            target_account: target.id,
            target_mailbox: target_inbox,
            target_message: None,
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
