//! The local-first mutation queue: enqueue atomicity, inverses, and the order
//! the queue comes back in after a restart.

use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    Account, BlobId, DraftId, Flag, FlagSet, MailboxId, MessageId, Operation, OperationId,
    OperationState, OperationTarget,
};
use postio_storage::bind;
use postio_storage::repository::OperationQueueRepository;
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

fn flags(raw: &str) -> FlagSet {
    raw.split_whitespace().map(Flag::parse).collect()
}

/// Inserts a message straight into the table: these tests are about the queue
/// beside the local write, not about the message repository.
async fn insert_message(connection: &Connection, mailbox: MailboxId) -> MessageId {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at)
             SELECT account_id, id, 0 FROM mailboxes WHERE id = ?1",
            [mailbox.get()],
        )
        .await
        .expect("insert a message");
    MessageId::new(connection.last_insert_rowid())
}

async fn set_seen(connection: &Connection, message: MessageId) {
    connection
        .execute(
            "UPDATE messages SET seen = 1, flags = '\\Seen' WHERE id = ?1",
            [message.get()],
        )
        .await
        .expect("flag the message locally");
}

async fn is_seen(connection: &Connection, message: MessageId) -> bool {
    postio_storage::sql::one(
        connection,
        "SELECT seen FROM messages WHERE id = ?1",
        bind![message.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
    )
    .await
    .expect("read the message")
        == 1
}

async fn has_pending_column(connection: &Connection, message: MessageId) -> bool {
    postio_storage::sql::one(
        connection,
        "SELECT has_pending_operations FROM messages WHERE id = ?1",
        bind![message.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
    )
    .await
    .expect("read the message")
        == 1
}

struct Fixture {
    account: Account,
    inbox: MailboxId,
    archive: MailboxId,
    trash: MailboxId,
}

async fn fixture(connection: &Connection) -> Fixture {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, "INBOX")
        .await
        .id;
    let archive = test_support::mailbox(connection, &account, "Archive")
        .await
        .id;
    let trash = test_support::mailbox(connection, &account, "Deleted Messages")
        .await
        .id;
    Fixture {
        account,
        inbox,
        archive,
        trash,
    }
}

// ---------------------------------------------------------------------------
// Enqueue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_enqueued_operation_round_trips_with_its_inverse() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let archive = Operation::Move {
        from: fixture.inbox,
        to: fixture.archive,
    };
    let queued = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &archive,
            at(9),
        )
        .await
        .expect("enqueue");

    assert!(queued.id.is_assigned());
    assert_eq!(queued.operation, archive);
    assert_eq!(
        queued.inverse,
        Some(Operation::Move {
            from: fixture.archive,
            to: fixture.inbox
        }),
        "carried on the row, so undo does not have to recompute it"
    );
    assert_eq!(queued.state, OperationState::Pending);
    assert_eq!(queued.attempts, 0);
    assert_eq!(queued.created_at, at(9));
    assert_eq!(queued.mailbox_id, Some(fixture.inbox));

    assert_eq!(queue.get(queued.id).await.expect("get"), Some(queued));
}

#[tokio::test]
async fn every_operation_type_survives_the_round_trip() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let operations = [
        Operation::SetFlags {
            flags: flags("\\Seen"),
        },
        Operation::ClearFlags {
            flags: flags("\\Seen \\Flagged"),
        },
        Operation::Move {
            from: fixture.inbox,
            to: fixture.archive,
        },
        Operation::Delete {
            from: fixture.inbox,
            trash: fixture.trash,
        },
        Operation::Expunge {
            mailbox: fixture.trash,
        },
        Operation::Append {
            mailbox: fixture.archive,
            blob: BlobId::new("abc123"),
            flags: flags("\\Seen"),
        },
        Operation::Send {
            draft: DraftId::new(1),
        },
    ];

    for operation in &operations {
        let queued = queue
            .enqueue(
                fixture.account.id,
                OperationTarget::Message(message),
                operation,
                at(9),
            )
            .await
            .expect("enqueue");
        let stored = queue.get(queued.id).await.expect("get").expect("the row");

        assert_eq!(&stored.operation, operation);
        assert_eq!(
            stored.inverse,
            operation.inverse(),
            "{} stored an inverse the model does not agree with",
            operation.op_type()
        );
    }

    assert_eq!(
        queue
            .pending(fixture.account.id, at(9))
            .await
            .expect("pending")
            .len(),
        operations.len()
    );
}

#[tokio::test]
async fn an_irreversible_operation_stores_no_inverse() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let queue = OperationQueueRepository::new(&connection);

    let queued = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Mailbox(fixture.trash),
            &Operation::Expunge {
                mailbox: fixture.trash,
            },
            at(9),
        )
        .await
        .expect("enqueue");

    assert_eq!(queued.inverse, None);
    assert!(!queued.is_undoable(), "the UI must not offer undo for it");
}

#[tokio::test]
async fn undoing_enqueues_the_inverse_down_the_same_path() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let archived = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::Move {
                from: fixture.inbox,
                to: fixture.archive,
            },
            at(9),
        )
        .await
        .expect("enqueue");

    let undo = queue
        .enqueue_inverse(&archived, at(10))
        .await
        .expect("undo");

    assert_eq!(
        undo.operation,
        Operation::Move {
            from: fixture.archive,
            to: fixture.inbox
        }
    );
    assert_eq!(undo.target, archived.target, "the same message");
    assert_eq!(undo.state, OperationState::Pending, "an ordinary queue row");
    assert!(undo.id.get() > archived.id.get(), "and it drains after it");
}

#[tokio::test]
async fn there_is_no_inverse_to_enqueue_for_an_irreversible_operation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let queue = OperationQueueRepository::new(&connection);

    let expunge = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Mailbox(fixture.trash),
            &Operation::Expunge {
                mailbox: fixture.trash,
            },
            at(9),
        )
        .await
        .expect("enqueue");

    assert!(matches!(
        queue.enqueue_inverse(&expunge, at(10)).await,
        Err(postio_storage::Error::NotUndoable { op_type }) if op_type == "expunge"
    ));
}

// ---------------------------------------------------------------------------
// Atomicity with the local write
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_local_write_and_the_enqueue_commit_together() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;

    let transaction = connection.transaction().await.expect("begin");
    set_seen(&transaction, message).await;
    OperationQueueRepository::new(&transaction)
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    transaction.commit().await.expect("commit");

    assert!(is_seen(&connection, message).await);
    assert_eq!(
        OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, at(9))
            .await
            .expect("pending")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_rolled_back_local_write_takes_its_operation_with_it() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;

    let transaction = connection.transaction().await.expect("begin");
    set_seen(&transaction, message).await;
    OperationQueueRepository::new(&transaction)
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    drop(transaction);

    assert!(
        !is_seen(&connection, message).await,
        "the local write is gone"
    );
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, at(9))
            .await
            .expect("pending")
            .is_empty(),
        "so the server must never be told about it"
    );
    assert!(!has_pending_column(&connection, message).await);
}

#[tokio::test]
async fn enqueueing_marks_the_message_as_having_work_outstanding() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    assert!(!has_pending_column(&connection, message).await);

    let queued = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    assert!(
        has_pending_column(&connection, message).await,
        "the list reads this column rather than joining the queue"
    );
    assert!(queue.has_pending(queued.target).await.expect("has_pending"));

    queue.delete(queued.id).await.expect("delete");

    assert!(!has_pending_column(&connection, message).await);
    assert!(!queue.has_pending(queued.target).await.expect("has_pending"));
}

// ---------------------------------------------------------------------------
// Enqueueing a named list at once
// ---------------------------------------------------------------------------
//
// The multi-select twin of `enqueue_set`: a selection built by clicking is a
// list of ids rather than a mailbox predicate, but it still must not cost one
// statement per message the way looping over `enqueue` does.

#[tokio::test]
async fn enqueueing_many_writes_one_row_per_message_naming_each_one() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let mut messages: Vec<MessageId> = Vec::new();
    for _ in 0..5 {
        messages.push(insert_message(&connection, fixture.inbox).await);
    }
    let queue = OperationQueueRepository::new(&connection);
    let archive = Operation::Move {
        from: fixture.inbox,
        to: fixture.archive,
    };

    queue
        .enqueue_many(fixture.account.id, &messages, &archive, at(9))
        .await
        .expect("enqueue many");

    let rows = queue
        .pending(fixture.account.id, at(9))
        .await
        .expect("pending");
    assert_eq!(
        rows.iter().map(|row| row.target).collect::<Vec<_>>(),
        messages
            .iter()
            .map(|id| OperationTarget::Message(*id))
            .collect::<Vec<_>>(),
        "every row names the message whose UID the drainer will need"
    );
    for row in &rows {
        assert_eq!(row.operation, archive);
        assert_eq!(row.state, OperationState::Pending);
        assert_eq!(row.mailbox_id, Some(fixture.inbox));
        assert_eq!(
            row.inverse,
            archive.inverse(),
            "the inverse is decided at enqueue time here as it is anywhere else"
        );
    }
}

#[tokio::test]
async fn enqueueing_many_marks_every_message_as_having_work_outstanding() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let mut messages: Vec<MessageId> = Vec::new();
    for _ in 0..3 {
        messages.push(insert_message(&connection, fixture.inbox).await);
    }
    let queue = OperationQueueRepository::new(&connection);

    for message in &messages {
        assert!(!has_pending_column(&connection, *message).await);
    }

    queue
        .enqueue_many(
            fixture.account.id,
            &messages,
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue many");

    for message in &messages {
        assert!(
            has_pending_column(&connection, *message).await,
            "the list reads this column rather than joining the queue"
        );
    }
}

#[tokio::test]
async fn enqueueing_many_with_no_ids_writes_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let queue = OperationQueueRepository::new(&connection);

    queue
        .enqueue_many(
            fixture.account.id,
            &[],
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue many, of nothing");

    assert!(
        queue
            .pending(fixture.account.id, at(9))
            .await
            .expect("pending")
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Order, and surviving a restart
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_queue_survives_a_restart_in_enqueue_order() {
    let database = test_support::temp().await;
    let account_id;
    let expected: Vec<Operation>;

    {
        let connection = database.connect().await.expect("checkout");
        let fixture = fixture(&connection).await;
        account_id = fixture.account.id;
        let message = insert_message(&connection, fixture.inbox).await;
        let queue = OperationQueueRepository::new(&connection);

        expected = vec![
            Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            Operation::Move {
                from: fixture.inbox,
                to: fixture.archive,
            },
            Operation::ClearFlags {
                flags: flags("\\Flagged"),
            },
            Operation::Delete {
                from: fixture.archive,
                trash: fixture.trash,
            },
        ];
        for (index, operation) in expected.iter().enumerate() {
            queue
                .enqueue(
                    account_id,
                    OperationTarget::Message(message),
                    operation,
                    at(9 + index as u32),
                )
                .await
                .expect("enqueue");
        }
    }

    // A new pool, and for a file-backed database a genuinely new connection.
    let reopened = postio_storage::Store::open(
        database.directory().join("postio.db"),
        &postio_storage::test_support::key(),
    )
    .await
    .expect("reopen");
    let connection = reopened.connect().await.expect("checkout");
    let drained: Vec<Operation> = OperationQueueRepository::new(&connection)
        .pending(account_id, at(20))
        .await
        .expect("pending")
        .into_iter()
        .map(|queued| queued.operation)
        .collect();

    assert_eq!(
        drained, expected,
        "the user performed these in this order and the server must see them that way"
    );
}

#[tokio::test]
async fn a_backed_off_operation_is_skipped_until_its_time() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let first = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    let second = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::ClearFlags {
                flags: flags("\\Flagged"),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    queue
        .defer(first.id, at(12), "connection reset")
        .await
        .expect("defer");

    let ready = queue
        .pending(fixture.account.id, at(10))
        .await
        .expect("pending");
    assert_eq!(
        ready.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![second.id],
        "the deferred row is not due yet"
    );

    let later = queue
        .pending(fixture.account.id, at(13))
        .await
        .expect("pending");
    assert_eq!(
        later.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![first.id, second.id],
        "and when it is due it goes back to its place in line"
    );
    let deferred = queue.get(first.id).await.expect("get").expect("the row");
    assert_eq!(deferred.attempts, 1);
    assert_eq!(deferred.last_error.as_deref(), Some("connection reset"));
}

/// A scheduled send (or anything else with a deliberate "not before" time)
/// uses the same skip-until-due gate a backed-off retry does, but must not
/// look like one: no attempt has been made yet, and nothing has failed.
#[tokio::test]
async fn a_scheduled_operation_is_skipped_until_its_send_time() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let immediate = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    let scheduled = queue
        .enqueue_not_before(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::ClearFlags {
                flags: flags("\\Flagged"),
            },
            at(9),
            at(15),
        )
        .await
        .expect("enqueue_not_before");

    let too_early = queue
        .pending(fixture.account.id, at(12))
        .await
        .expect("pending");
    assert_eq!(
        too_early.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![immediate.id],
        "the scheduled row is not due yet"
    );

    let due = queue
        .pending(fixture.account.id, at(15))
        .await
        .expect("pending");
    assert_eq!(
        due.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![immediate.id, scheduled.id],
        "and once its time arrives it drains in its enqueue-order place"
    );

    let row = queue
        .get(scheduled.id)
        .await
        .expect("get")
        .expect("the row");
    assert_eq!(
        row.next_attempt_at,
        Some(at(15)),
        "the not-before time is stored verbatim, restart-safe in the row itself"
    );
    assert_eq!(row.attempts, 0, "no attempt has been made yet");
    assert_eq!(row.last_error, None, "nothing has failed");
    assert_eq!(row.state, OperationState::Pending);
}

#[tokio::test]
async fn an_operation_left_in_flight_by_a_crash_is_retried_rather_than_dropped() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let queued = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    queue
        .mark_in_flight(queued.id, at(9))
        .await
        .expect("in flight");

    assert!(
        queue
            .pending(fixture.account.id, at(10))
            .await
            .expect("pending")
            .is_empty(),
        "it is somebody else's now"
    );

    // The crash, and the next start.
    let recovered = queue
        .requeue_in_flight(fixture.account.id, at(11))
        .await
        .expect("requeue");

    assert_eq!(recovered, 1);
    assert_eq!(
        queue
            .pending(fixture.account.id, at(11))
            .await
            .expect("pending")
            .len(),
        1,
        "the server may or may not have applied it; operations are idempotent"
    );
}

#[tokio::test]
async fn a_settled_operation_stops_appearing_in_the_queue() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let message = insert_message(&connection, fixture.inbox).await;
    let queue = OperationQueueRepository::new(&connection);

    let done = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .await
        .expect("enqueue");
    let failed = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::ClearFlags {
                flags: flags("\\Flagged"),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    queue.mark_done(done.id, at(10)).await.expect("done");
    queue
        .mark_failed(failed.id, at(10), "no such mailbox")
        .await
        .expect("failed");

    assert!(
        queue
            .pending(fixture.account.id, at(11))
            .await
            .expect("pending")
            .is_empty()
    );
    assert!(
        !has_pending_column(&connection, message).await,
        "and the message stops advertising outstanding work"
    );
    assert_eq!(
        queue
            .get(failed.id)
            .await
            .expect("get")
            .expect("the row")
            .state,
        OperationState::Failed,
        "a failure is kept so the user can be told about it"
    );
}

#[tokio::test]
async fn operations_for_another_account_are_never_drained_together() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let first = fixture(&connection).await;
    let second = fixture(&connection).await;
    let queue = OperationQueueRepository::new(&connection);

    queue
        .enqueue(
            first.account.id,
            OperationTarget::Mailbox(first.inbox),
            &Operation::Expunge {
                mailbox: first.inbox,
            },
            at(9),
        )
        .await
        .expect("enqueue");

    assert_eq!(
        queue
            .pending(first.account.id, at(9))
            .await
            .expect("pending")
            .len(),
        1
    );
    assert!(
        queue
            .pending(second.account.id, at(9))
            .await
            .expect("pending")
            .is_empty()
    );
}

#[tokio::test]
async fn reading_an_operation_that_is_not_there_is_none() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let queue = OperationQueueRepository::new(&connection);

    assert_eq!(queue.get(OperationId::new(404)).await.expect("get"), None);
    assert!(!queue.delete(OperationId::new(404)).await.expect("delete"));
}

#[tokio::test]
async fn deleting_an_account_takes_its_queue_with_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let fixture = fixture(&connection).await;
    let queue = OperationQueueRepository::new(&connection);
    queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Mailbox(fixture.inbox),
            &Operation::Expunge {
                mailbox: fixture.inbox,
            },
            at(9),
        )
        .await
        .expect("enqueue");

    connection
        .execute(
            "DELETE FROM accounts WHERE id = ?1",
            [fixture.account.id.get()],
        )
        .await
        .expect("delete the account");

    assert!(
        queue
            .pending(fixture.account.id, at(9))
            .await
            .expect("pending")
            .is_empty()
    );
}
