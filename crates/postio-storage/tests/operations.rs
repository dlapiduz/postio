//! The local-first mutation queue: enqueue atomicity, inverses, and the order
//! the queue comes back in after a restart.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;

use postio_model::{
    Account, BlobId, DraftId, Flag, FlagSet, MailboxId, MessageId, Operation, OperationId,
    OperationState, OperationTarget,
};
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
fn insert_message(connection: &Connection, mailbox: MailboxId) -> MessageId {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at)
             SELECT account_id, id, 0 FROM mailboxes WHERE id = ?1",
            [mailbox.get()],
        )
        .expect("insert a message");
    MessageId::new(connection.last_insert_rowid())
}

fn set_seen(connection: &Connection, message: MessageId) {
    connection
        .execute(
            "UPDATE messages SET seen = 1, flags = '\\Seen' WHERE id = ?1",
            [message.get()],
        )
        .expect("flag the message locally");
}

fn is_seen(connection: &Connection, message: MessageId) -> bool {
    connection
        .query_row(
            "SELECT seen FROM messages WHERE id = ?1",
            [message.get()],
            |row| row.get::<_, i64>(0),
        )
        .expect("read the message")
        == 1
}

fn has_pending_column(connection: &Connection, message: MessageId) -> bool {
    connection
        .query_row(
            "SELECT has_pending_operations FROM messages WHERE id = ?1",
            [message.get()],
            |row| row.get::<_, i64>(0),
        )
        .expect("read the message")
        == 1
}

struct Fixture {
    account: Account,
    inbox: MailboxId,
    archive: MailboxId,
    trash: MailboxId,
}

fn fixture(connection: &Connection) -> Fixture {
    let account = test_support::account(connection);
    let inbox = test_support::mailbox(connection, &account, "INBOX").id;
    let archive = test_support::mailbox(connection, &account, "Archive").id;
    let trash = test_support::mailbox(connection, &account, "Deleted Messages").id;
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

#[test]
fn an_enqueued_operation_round_trips_with_its_inverse() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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

    assert_eq!(queue.get(queued.id).expect("get"), Some(queued));
}

#[test]
fn every_operation_type_survives_the_round_trip() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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
            .expect("enqueue");
        let stored = queue.get(queued.id).expect("get").expect("the row");

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
            .expect("pending")
            .len(),
        operations.len()
    );
}

#[test]
fn an_irreversible_operation_stores_no_inverse() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
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
        .expect("enqueue");

    assert_eq!(queued.inverse, None);
    assert!(!queued.is_undoable(), "the UI must not offer undo for it");
}

#[test]
fn undoing_enqueues_the_inverse_down_the_same_path() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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
        .expect("enqueue");

    let undo = queue.enqueue_inverse(&archived, at(10)).expect("undo");

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

#[test]
fn there_is_no_inverse_to_enqueue_for_an_irreversible_operation() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
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
        .expect("enqueue");

    assert!(matches!(
        queue.enqueue_inverse(&expunge, at(10)),
        Err(postio_storage::Error::NotUndoable { op_type }) if op_type == "expunge"
    ));
}

// ---------------------------------------------------------------------------
// Atomicity with the local write
// ---------------------------------------------------------------------------

#[test]
fn the_local_write_and_the_enqueue_commit_together() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);

    let transaction = connection.transaction().expect("begin");
    set_seen(&transaction, message);
    OperationQueueRepository::new(&transaction)
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .expect("enqueue");
    transaction.commit().expect("commit");

    assert!(is_seen(&connection, message));
    assert_eq!(
        OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, at(9))
            .expect("pending")
            .len(),
        1
    );
}

#[test]
fn a_rolled_back_local_write_takes_its_operation_with_it() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);

    let transaction = connection.transaction().expect("begin");
    set_seen(&transaction, message);
    OperationQueueRepository::new(&transaction)
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .expect("enqueue");
    drop(transaction);

    assert!(!is_seen(&connection, message), "the local write is gone");
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(fixture.account.id, at(9))
            .expect("pending")
            .is_empty(),
        "so the server must never be told about it"
    );
    assert!(!has_pending_column(&connection, message));
}

#[test]
fn enqueueing_marks_the_message_as_having_work_outstanding() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
    let queue = OperationQueueRepository::new(&connection);

    assert!(!has_pending_column(&connection, message));

    let queued = queue
        .enqueue(
            fixture.account.id,
            OperationTarget::Message(message),
            &Operation::SetFlags {
                flags: flags("\\Seen"),
            },
            at(9),
        )
        .expect("enqueue");

    assert!(
        has_pending_column(&connection, message),
        "the list reads this column rather than joining the queue"
    );
    assert!(queue.has_pending(queued.target).expect("has_pending"));

    queue.delete(queued.id).expect("delete");

    assert!(!has_pending_column(&connection, message));
    assert!(!queue.has_pending(queued.target).expect("has_pending"));
}

// ---------------------------------------------------------------------------
// Order, and surviving a restart
// ---------------------------------------------------------------------------

#[test]
fn the_queue_survives_a_restart_in_enqueue_order() {
    let database = test_support::temp();
    let account_id;
    let expected: Vec<Operation>;

    {
        let connection = database.connection().expect("checkout");
        let fixture = fixture(&connection);
        account_id = fixture.account.id;
        let message = insert_message(&connection, fixture.inbox);
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
                .expect("enqueue");
        }
    }

    // A new pool, and for a file-backed database a genuinely new connection.
    let reopened =
        postio_storage::Database::open(database.directory().join("postio.db")).expect("reopen");
    let connection = reopened.connection().expect("checkout");
    let drained: Vec<Operation> = OperationQueueRepository::new(&connection)
        .pending(account_id, at(20))
        .expect("pending")
        .into_iter()
        .map(|queued| queued.operation)
        .collect();

    assert_eq!(
        drained, expected,
        "the user performed these in this order and the server must see them that way"
    );
}

#[test]
fn a_backed_off_operation_is_skipped_until_its_time() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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
        .expect("enqueue");

    queue
        .defer(first.id, at(12), "connection reset")
        .expect("defer");

    let ready = queue.pending(fixture.account.id, at(10)).expect("pending");
    assert_eq!(
        ready.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![second.id],
        "the deferred row is not due yet"
    );

    let later = queue.pending(fixture.account.id, at(13)).expect("pending");
    assert_eq!(
        later.iter().map(|queued| queued.id).collect::<Vec<_>>(),
        vec![first.id, second.id],
        "and when it is due it goes back to its place in line"
    );
    let deferred = queue.get(first.id).expect("get").expect("the row");
    assert_eq!(deferred.attempts, 1);
    assert_eq!(deferred.last_error.as_deref(), Some("connection reset"));
}

#[test]
fn an_operation_left_in_flight_by_a_crash_is_retried_rather_than_dropped() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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
        .expect("enqueue");
    queue.mark_in_flight(queued.id, at(9)).expect("in flight");

    assert!(
        queue
            .pending(fixture.account.id, at(10))
            .expect("pending")
            .is_empty(),
        "it is somebody else's now"
    );

    // The crash, and the next start.
    let recovered = queue
        .requeue_in_flight(fixture.account.id, at(11))
        .expect("requeue");

    assert_eq!(recovered, 1);
    assert_eq!(
        queue
            .pending(fixture.account.id, at(11))
            .expect("pending")
            .len(),
        1,
        "the server may or may not have applied it; operations are idempotent"
    );
}

#[test]
fn a_settled_operation_stops_appearing_in_the_queue() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
    let message = insert_message(&connection, fixture.inbox);
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
        .expect("enqueue");

    queue.mark_done(done.id, at(10)).expect("done");
    queue
        .mark_failed(failed.id, at(10), "no such mailbox")
        .expect("failed");

    assert!(
        queue
            .pending(fixture.account.id, at(11))
            .expect("pending")
            .is_empty()
    );
    assert!(
        !has_pending_column(&connection, message),
        "and the message stops advertising outstanding work"
    );
    assert_eq!(
        queue.get(failed.id).expect("get").expect("the row").state,
        OperationState::Failed,
        "a failure is kept so the user can be told about it"
    );
}

#[test]
fn operations_for_another_account_are_never_drained_together() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let first = fixture(&connection);
    let second = fixture(&connection);
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
        .expect("enqueue");

    assert_eq!(
        queue
            .pending(first.account.id, at(9))
            .expect("pending")
            .len(),
        1
    );
    assert!(
        queue
            .pending(second.account.id, at(9))
            .expect("pending")
            .is_empty()
    );
}

#[test]
fn reading_an_operation_that_is_not_there_is_none() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let queue = OperationQueueRepository::new(&connection);

    assert_eq!(queue.get(OperationId::new(404)).expect("get"), None);
    assert!(!queue.delete(OperationId::new(404)).expect("delete"));
}

#[test]
fn deleting_an_account_takes_its_queue_with_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let fixture = fixture(&connection);
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
        .expect("enqueue");

    connection
        .execute(
            "DELETE FROM accounts WHERE id = ?1",
            [fixture.account.id.get()],
        )
        .expect("delete the account");

    assert!(
        queue
            .pending(fixture.account.id, at(9))
            .expect("pending")
            .is_empty()
    );
}
