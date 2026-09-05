//! The cross-account move saga, drained (#188, ADR 0005 Q9).
//!
//! Each design constraint the ADR states is a test here, against two
//! `MockBackend`s standing in for two servers and one store holding both
//! accounts. What the matrix is really about: **however the two drainers
//! interleave, replay, or die, the source copy outlives anything short of a
//! confirmed copy in the target.**

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

/// #531's last criterion: the inverse saga's removal reaches the **target**
/// server, using the **target's** coordinates.
///
/// This is the failure the whole issue is shaped around, and it is a silent
/// one: an inverse removal that names a coordinate the target server does
/// not have reaches nothing, expunges nothing, and settles `done` reporting
/// success. The user is told their message came back and account B keeps its
/// copy for ever.
///
/// It is driven the whole way rather than staged: the forward saga runs
/// against both mock servers until the move is genuinely complete, the undo
/// builds the inverse the way `u` does, and then the inverse's two halves
/// drain against the same two servers. Every coordinate in play is one a
/// server actually issued.
#[tokio::test]
async fn the_inverse_removal_reaches_the_target_server_with_its_own_coordinates() {
    let world = world(RAW, true);
    let source = source_server().await;
    let target = target_server(true).await;

    // ── the move, all the way ────────────────────────────────────────────
    drain(&world, &target, world.target_account).await;
    drain(&world, &source, world.source_account).await;
    assert_eq!(phase(&world), MovePhase::Done);
    assert_eq!(messages_in(&source, "INBOX").await, 0, "A gave it up");
    assert_eq!(messages_in(&target, "INBOX").await, 1, "B has it");

    // The identity B's server issued, now on the row (this branch's earlier
    // commit) — and the coordinate the inverse must remove by.
    let copy_identity = {
        let connection = world.database.connection().expect("checkout");
        MessageRepository::new(&connection)
            .get(world.target_message)
            .expect("read")
            .expect("the copy")
            .server
            .remote_id
            .expect("phase 2 wrote what the append proved")
    };

    // ── the undo, built the way `u` builds it ────────────────────────────
    let forward_blob = {
        let connection = world.database.connection().expect("checkout");
        CrossAccountMoveRepository::new(&connection)
            .get(world.saga)
            .expect("read")
            .expect("the forward saga")
            .raw_blob_id
    };
    let inverse = {
        let connection = world.database.connection().expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let inverse = sagas
            .create(&NewCrossAccountMove {
                source_message: world.target_message,
                source_account: world.target_account,
                source_mailbox: world.target_inbox,
                target_account: world.source_account,
                target_mailbox: world.source_inbox,
                target_message: Some(world.source_message),
                // The bytes to put back. Content-addressed, so this is the
                // same blob the forward pass appended to B — `invert_one`
                // reads it off the copy row for the same reason.
                raw_blob_id: forward_blob.clone(),
                rfc_message_id: Some("<engine@example.com>".to_owned()),
            })
            .expect("the inverse saga");
        let queue = OperationQueueRepository::new(&connection);
        queue
            .enqueue(
                world.source_account,
                OperationTarget::Message(world.source_message),
                &Operation::CrossAccountCopy { saga: inverse },
                at(11),
            )
            .expect("enqueue the copy back");
        // Enqueued while the copy still carries B's identity, which is what
        // `source_remote_id` snapshots — and what phase 3 removes by.
        queue
            .enqueue(
                world.target_account,
                OperationTarget::Message(world.target_message),
                &Operation::CrossAccountRemove { saga: inverse },
                at(11),
            )
            .expect("enqueue the removal from B");
        MessageRepository::new(&connection)
            .set_deleted_locally(&[world.source_message], false)
            .expect("the original comes back");
        inverse
    };

    // Phase 1 of the inverse, on account A. Unlike the `confirmed` case,
    // `done` means A genuinely gave the message up — so this is a real
    // append, from the raw blob the forward pass already stored.
    drain_at(&world, &source, world.source_account, 12).await;
    let phase_now = {
        let connection = world.database.connection().expect("checkout");
        CrossAccountMoveRepository::new(&connection)
            .get(inverse)
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

    // ── and the half this test exists for ────────────────────────────────
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

    // The count above *is* the coordinate assertion, which is why there is
    // no separate one. A `RemoteId` from account A's UID space either fails
    // `wire_uid`'s generation check outright or names a UID B never issued;
    // either way the expunge matches nothing and the message is still there.
    // The only way this inbox reaches zero is the removal naming what B's
    // own server assigned — which is `copy_identity`, read off the row
    // before the undo, and written there by phase 2.
    let _ = copy_identity;
}

#[tokio::test]
async fn a_copy_replayed_after_the_saga_already_aborted_is_obsolete() {
    // A restart that lost its settle re-queues the operation the same way
    // `a_replayed_copy_finds_the_first_copy_and_makes_no_second` does -- but
    // this time onto a saga that is already `Aborted` (Q13's own vanished-
    // destination case), which `copy` must recognise before it ever asks
    // whether the destination exists again.
    let world = world(RAW, true);
    let target = target_server(true).await;
    {
        let connection = world.database.connection().expect("checkout");
        postio_storage::repository::MailboxRepository::new(&connection)
            .delete(world.target_inbox)
            .expect("the destination goes away");
    }

    drain(&world, &target, world.target_account).await;
    assert_eq!(phase(&world), MovePhase::Aborted);

    {
        let connection = world.database.connection().expect("checkout");
        connection
            .execute(
                "UPDATE operation_queue SET state = 'pending' WHERE account_id = ?1",
                [world.target_account.get()],
            )
            .expect("re-queue the copy as a restart would find it");
    }
    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world),
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
    // The Message-ID search is what proves an append without UIDPLUS -- and
    // without UIDPLUS it also runs first, before any append is attempted, so
    // a server that refuses it must be retried rather than treated as a
    // permanent failure: nothing has been uploaded yet to fail over.
    let world = world(RAW, true);
    let target = target_server(false).await;
    target.inject_after(
        1,
        Fault::Rejected("SEARCH not permitted just now".to_owned()),
    );

    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world),
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
    // No Message-ID to search by, so the append is the very first thing
    // `copy` asks the server for. A server that refuses it is a reason to
    // retry later, not to abandon the move -- the source copy is still
    // exactly where it was.
    let world = world(RAW_ANONYMOUS, false);
    let target = target_server(false).await;
    target.inject_after(1, Fault::Rejected("APPEND refused".to_owned()));

    drain(&world, &target, world.target_account).await;

    assert_eq!(
        phase(&world),
        MovePhase::Copying,
        "an upload failure must leave the saga exactly where it was"
    );
    assert_eq!(messages_in(&target, "INBOX").await, 0);
}

/// A message with no server identity at all, in an account of its own --
/// the case #211-236 exists for and neither existing removal test reaches:
/// `enqueue`'s own snapshot is empty (there was nothing to snapshot) *and*
/// the live row has nothing either, so `remove` has no coordinate from
/// either source and must settle the saga without asking a server anything.
#[tokio::test]
async fn a_removal_with_no_coordinate_anywhere_settles_without_reaching_a_server() {
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

    // No `server.remote_id` at all -- born locally with nowhere on a server
    // it has ever been, which is the state #940 describes for a provisional
    // copy and, degenerately, the state any message has before its first
    // sync.
    let mut message = Message::new(source.id, source_inbox, at(8));
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
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
        .expect("saga");
    // `remove` refuses to run before `confirmed` -- reachable straight from
    // `copying`, per the phase graph, without staging a whole copy this test
    // is not about.
    CrossAccountMoveRepository::new(&connection)
        .transition(saga, MovePhase::Confirmed)
        .expect("confirmed");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            source.id,
            OperationTarget::Message(source_message),
            &Operation::CrossAccountRemove { saga },
            at(9),
        )
        .expect("enqueue remove");
    drop(connection);

    let source_backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").uid_validity(UidValidity::new(1)))
        .build();
    source_backend.connect().await.expect("connect");

    let connection = database.connection().expect("checkout");
    Drainer::new(&source_backend)
        .with_blobs(&blobs)
        .drain(&connection, source.id, at(10))
        .await
        .expect("a drain pass");

    let phase = CrossAccountMoveRepository::new(&connection)
        .get(saga)
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
    // Phase 1 needs the raw bytes on this machine, and a queue row can
    // outrun a backfill that has not landed them yet. Nothing to upload is
    // a reason to wait, not to fail the move over a race with a different
    // pass of the same engine.
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
    message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
    let source_message = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("source message");
    let mut copy = message.clone();
    copy.id = MessageId::UNASSIGNED;
    copy.account_id = target.id;
    copy.mailbox_id = target_inbox;
    copy.server = postio_model::ServerIdentifiers::default();
    let target_message = MessageRepository::new(&connection)
        .create(&mut copy)
        .expect("the provisional copy");

    // No `raw_blob_id` at all -- the state a queue row is in the moment it
    // is enqueued, before the backfill that will eventually fetch this
    // message's bytes has had a turn.
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
    drop(connection);

    let target_backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    target_backend.connect().await.expect("connect");

    let connection = database.connection().expect("checkout");
    Drainer::new(&target_backend)
        .with_blobs(&blobs)
        .drain(&connection, target.id, at(10))
        .await
        .expect("a drain pass");

    let phase = CrossAccountMoveRepository::new(&connection)
        .get(saga)
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
