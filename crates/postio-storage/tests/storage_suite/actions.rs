use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;
use postio_storage::sql::bind;

use postio_model::{Flag, MailboxId, Message, MessageId, Operation, OperationTarget};
use postio_storage::actions::{Relocation, relocate, set_flag};
use postio_storage::repository::{MessageRepository, OperationQueueRepository};
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 4, hour, 0, 0).unwrap()
}

#[tokio::test]
async fn two_relocations_share_one_caller_owned_transaction() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let first = a_message(&connection, inbox, "uid-1").await;
    let second = a_message(&connection, inbox, "uid-2").await;

    let transaction = connection.transaction().await.expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![first])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(9),
    )
    .await
    .expect("the first relocation");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![second])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(10),
    )
    .await
    .expect("the second relocation, in the same transaction");
    transaction.commit().await.expect("commit");

    let messages = MessageRepository::new(&connection);
    for id in [first, second] {
        let row = messages
            .get(id)
            .await
            .expect("read back")
            .expect("still there");
        assert_eq!(
            row.mailbox_id, archive,
            "both relocations should have landed once the caller committed"
        );
    }
}

/// The queue row is written before the rows move: `move_to` clears `remote_id`, so enqueueing after
/// the move leaves a `None` snapshot, which can never be pushed (#289).
#[tokio::test]
async fn the_queue_row_carries_the_messages_server_identity() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let message = a_message(&connection, inbox, "uid-9").await;

    let transaction = connection.transaction().await.expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![message])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(9),
    )
    .await
    .expect("relocate");
    transaction.commit().await.expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .await
        .expect("read the queue");
    assert_eq!(queued.len(), 1, "one operation for one move");
    assert!(
        matches!(
            queued[0].operation,
            Operation::Move { from, to } if from == inbox && to == archive
        ),
        "expected a Move from the source mailbox, got {:?}",
        queued[0].operation
    );
    assert_eq!(
        queued[0].source_remote_id.as_ref().map(|id| id.as_str()),
        Some("uid-9"),
        "the queue row should carry the message's server identity; a NULL \
         here leaves the drain with nothing to address the server with"
    );
}

#[tokio::test]
async fn a_trash_relocation_enqueues_a_delete() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let trash = test_support::mailbox(&connection, &account, "Trash")
        .await
        .id;
    let message = a_message(&connection, inbox, "uid-9").await;

    let transaction = connection.transaction().await.expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![message])].into_iter().collect(),
        trash,
        Relocation::Trash,
        at(9),
    )
    .await
    .expect("relocate");
    transaction.commit().await.expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .await
        .expect("read the queue");
    assert!(
        matches!(
            queued[0].operation,
            Operation::Delete { from, trash: to } if from == inbox && to == trash
        ),
        "expected a Delete carrying the trash mailbox, got {:?}",
        queued[0].operation
    );
}

/// A verb that committed internally would leave the move behind when the sync pass fails. The
/// rollback is an `Err` out of the scope, since `Drop` cannot await.
#[tokio::test]
async fn a_rolled_back_transaction_relocates_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let message = a_message(&connection, inbox, "uid-9").await;

    let rolled_back: Result<(), postio_storage::Error> =
        postio_storage::transaction(&connection, async |transaction| {
            relocate(
                &transaction,
                account.id,
                &[(inbox, vec![message])].into_iter().collect(),
                archive,
                Relocation::Move,
                at(9),
            )
            .await
            .expect("relocate");
            Err(postio_storage::Error::NotFound {
                entity: "a reason to roll back",
                id: 0,
            })
        })
        .await;
    assert!(rolled_back.is_err(), "the scope returns the caller's error");

    let row = MessageRepository::new(&connection)
        .get(message)
        .await
        .expect("read back")
        .expect("still there");
    assert_eq!(
        row.mailbox_id, inbox,
        "a rolled-back transaction must leave the message where it was"
    );
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(account.id, at(23))
            .await
            .expect("read the queue")
            .is_empty(),
        "and must enqueue nothing"
    );
}

/// The `remote_id` is the coordinate `enqueue_many` snapshots and `move_to` nulls, so it tells the
/// two orderings apart (#289).
async fn a_message(connection: &Connection, mailbox: MailboxId, remote: &str) -> MessageId {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, remote_id)
             SELECT account_id, id, 0, 0, ?2 FROM mailboxes WHERE id = ?1",
            bind![mailbox.get(), remote],
        )
        .await
        .expect("insert a message");
    MessageId::new(connection.last_insert_rowid())
}

#[tokio::test]
async fn two_flag_changes_share_one_caller_owned_transaction() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let first = a_message(&connection, inbox, "uid-1").await;
    let second = a_message(&connection, inbox, "uid-2").await;
    let rows = read(&connection, &[first, second]).await;

    let transaction = connection.transaction().await.expect("open a transaction");
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        true,
        at(9),
    )
    .await
    .expect("the first flag change");
    set_flag(
        &transaction,
        account.id,
        &[&rows[1]],
        &Flag::Seen,
        true,
        at(10),
    )
    .await
    .expect("the second, in the same transaction");
    transaction.commit().await.expect("commit");

    for id in [first, second] {
        let row = MessageRepository::new(&connection)
            .get(id)
            .await
            .expect("read back")
            .expect("still there");
        assert!(
            row.flags.contains(&Flag::Seen),
            "both flag changes should have landed once the caller committed"
        );
    }
}

#[tokio::test]
async fn clearing_a_flag_enqueues_the_opposite_operation() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let message = a_message(&connection, inbox, "uid-9").await;

    let transaction = connection.transaction().await.expect("open a transaction");
    let rows = read(&transaction, &[message]).await;
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        true,
        at(9),
    )
    .await
    .expect("set");
    let rows = read(&transaction, &[message]).await;
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        false,
        at(10),
    )
    .await
    .expect("clear");
    transaction.commit().await.expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .await
        .expect("read the queue");
    assert_eq!(queued.len(), 2, "one operation per change");
    assert!(
        matches!(queued[0].operation, Operation::SetFlags { .. }),
        "the first change should enqueue SetFlags, got {:?}",
        queued[0].operation
    );
    assert!(
        matches!(queued[1].operation, Operation::ClearFlags { .. }),
        "the second should enqueue ClearFlags, got {:?}",
        queued[1].operation
    );
}

async fn read(connection: &Connection, ids: &[MessageId]) -> Vec<Message> {
    let messages = MessageRepository::new(connection);
    let mut collected = Vec::new();
    for id in ids.iter() {
        collected.push(
            messages
                .get(*id)
                .await
                .expect("read a message")
                .expect("the message exists"),
        );
    }
    collected
}

#[tokio::test]
async fn refiling_swaps_the_old_reason_for_the_new_one_and_tells_the_server_both() {
    use postio_storage::actions::mark_filtered;
    use postio_storage::repository::{FilterReason, FlagSource};
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let spam = a_message(&connection, inbox, "uid-1").await;
    let plain = a_message(&connection, inbox, "uid-2").await;
    let flags = [Flag::Seen, FilterReason::Spam.keyword()]
        .into_iter()
        .collect();
    MessageRepository::new(&connection)
        .set_flags(spam, &flags, FlagSource::Local)
        .await
        .expect("flag it");
    let rows = read(&connection, &[spam, plain]).await;

    let transaction = connection.transaction().await.expect("open a transaction");
    mark_filtered(
        &transaction,
        account.id,
        &[&rows[0], &rows[1]],
        Some(FilterReason::Promotion),
        at(9),
    )
    .await
    .expect("mark");
    transaction.commit().await.expect("commit");

    for row in read(&connection, &[spam, plain]).await {
        assert_eq!(
            FilterReason::in_flags(&row.flags),
            Some(FilterReason::Promotion),
            "one reason per message"
        );
    }
    assert!(
        read(&connection, &[spam]).await[0]
            .flags
            .contains(&Flag::Seen)
    );
    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .await
        .expect("read the queue");
    let told: Vec<(OperationTarget, bool)> = queued
        .iter()
        .map(|row| match &row.operation {
            Operation::ClearFlags { flags } => {
                (row.target, flags.contains(&FilterReason::Spam.keyword()))
            }
            Operation::SetFlags { flags } => (
                row.target,
                flags.contains(&FilterReason::Promotion.keyword()),
            ),
            other => panic!("only flag changes, got {other:?}"),
        })
        .collect();
    assert_eq!(
        told,
        [
            (OperationTarget::Message(spam), true),
            (OperationTarget::Message(spam), true),
            (OperationTarget::Message(plain), true)
        ],
        "spam's old reason is cleared, and both get the new one"
    );
}

#[tokio::test]
async fn unfiling_a_message_with_no_reason_changes_and_queues_nothing() {
    use postio_storage::actions::mark_filtered;
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let plain = a_message(&connection, inbox, "uid-1").await;
    let rows = read(&connection, &[plain]).await;

    let transaction = connection.transaction().await.expect("open a transaction");
    mark_filtered(&transaction, account.id, &[&rows[0]], None, at(9))
        .await
        .expect("mark");
    transaction.commit().await.expect("commit");

    assert_eq!(read(&connection, &[plain]).await[0].flags, rows[0].flags);
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(account.id, at(23))
            .await
            .expect("read the queue")
            .is_empty()
    );
}
