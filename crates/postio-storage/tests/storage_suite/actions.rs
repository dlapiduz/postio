//! The mutating verbs, called inside a transaction the caller owns.
//!
//! ADR 0028: the command bus and the rules pass must call **one**
//! implementation of each action, because ADR 0008 Q5 forbids a rules-only
//! mutation path. The seam is that the row write and its queue row move down
//! here, while resolving what the user aimed at, pushing undo and emitting
//! events stay in `postio_session::actions`.
//!
//! What decides the signature is ADR 0008 Q3: a header-only rule runs "in the
//! same transaction as the insert, before any event is emitted", and the sync
//! pass owns that transaction. So a verb here must accept one rather than
//! open one. That is the whole point of the first test below — a verb that
//! opened its own transaction would pass every outcome assertion in this file
//! and still be unusable by the caller it exists for.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{AccountId, Flag, Label, MailboxId, Message, MessageId, Operation};
use postio_storage::actions::{Relocation, relocate, set_flag, set_label};
use postio_storage::repository::{LabelRepository, MessageRepository, OperationQueueRepository};
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 4, hour, 0, 0).unwrap()
}

/// Two relocations, one transaction, and the caller decides when it commits.
///
/// This is what the rules pass does: a message arrives, a rule files it, and
/// the insert that brought it in has not committed yet. Nothing about the
/// outcome distinguishes this from two separate transactions -- the rows land
/// either way -- so the assertion that carries the weight is that the verb
/// *accepted* a borrowed transaction at all, and that nothing is visible
/// until the caller commits it.
#[test]
fn two_relocations_share_one_caller_owned_transaction() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;
    let first = a_message(&connection, inbox, "uid-1");
    let second = a_message(&connection, inbox, "uid-2");

    let transaction = connection.transaction().expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![first])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(9),
    )
    .expect("the first relocation");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![second])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(10),
    )
    .expect("the second relocation, in the same transaction");
    transaction.commit().expect("commit");

    let messages = MessageRepository::new(&connection);
    for id in [first, second] {
        let row = messages.get(id).expect("read back").expect("still there");
        assert_eq!(
            row.mailbox_id, archive,
            "both relocations should have landed once the caller committed"
        );
    }
}

/// The queue row carries the message's server identity.
///
/// Named for what it can actually prove. It was written as "the queue row is
/// written before the rows move" and asserted on the operation's `from`,
/// which is passed explicitly and reads the same either way -- it passed with
/// the two statements deliberately swapped. Checking the snapshot instead
/// does not rescue it: `enqueue_many` snapshots `remote_id` and `move_to`
/// nulls `uid`, `uid_validity` and `mod_seq`, so the orders are equivalent
/// for the data and nothing here can distinguish them.
///
/// What it does prove is that the enqueue happens at all and that the
/// coordinate reaches the queue row, which is what the drain addresses the
/// server with (#289).
#[test]
fn the_queue_row_carries_the_messages_server_identity() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![message])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(9),
    )
    .expect("relocate");
    transaction.commit().expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
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
    // `from` above is passed to the operation explicitly, so it would read
    // the same even if nothing were snapshotted. This is the part that would
    // go quiet if `enqueue_many` stopped capturing the coordinate.
    assert_eq!(
        queued[0].source_remote_id.as_ref().map(|id| id.as_str()),
        Some("uid-9"),
        "the queue row should carry the message's server identity; a NULL \
         here leaves the drain with nothing to address the server with"
    );
}

/// A trash relocation enqueues `Delete`, not `Move`.
///
/// The distinction is the server operation, and it is the only thing the
/// caller's `Relocation` chooses. `UndoKind` stays in `postio-session`:
/// nothing down here knows what an undo entry is.
#[test]
fn a_trash_relocation_enqueues_a_delete() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let trash = test_support::mailbox(&connection, &account, "Trash").id;
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![message])].into_iter().collect(),
        trash,
        Relocation::Trash,
        at(9),
    )
    .expect("relocate");
    transaction.commit().expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
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

/// Nothing lands if the caller rolls back.
///
/// The other half of "the caller owns the transaction": a rule whose sync
/// pass fails must leave no trace, and a verb that committed internally would
/// leave the move behind with the insert rolled back.
#[test]
fn a_rolled_back_transaction_relocates_nothing() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    relocate(
        &transaction,
        account.id,
        &[(inbox, vec![message])].into_iter().collect(),
        archive,
        Relocation::Move,
        at(9),
    )
    .expect("relocate");
    drop(transaction);

    let row = MessageRepository::new(&connection)
        .get(message)
        .expect("read back")
        .expect("still there");
    assert_eq!(
        row.mailbox_id, inbox,
        "a rolled-back transaction must leave the message where it was"
    );
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(account.id, at(23))
            .expect("read the queue")
            .is_empty(),
        "and must enqueue nothing"
    );
}

/// A message straight into `mailbox`, carrying a server identity.
///
/// The `remote_id` is not decoration: it is the coordinate `enqueue_many`
/// snapshots and `move_to` nulls, so it is the only thing that can tell the
/// two orderings apart (#289).
fn a_message(connection: &rusqlite::Connection, mailbox: MailboxId, remote: &str) -> MessageId {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, remote_id)
             SELECT account_id, id, 0, ?2 FROM mailboxes WHERE id = ?1",
            rusqlite::params![mailbox.get(), remote],
        )
        .expect("insert a message");
    MessageId::new(connection.last_insert_rowid())
}

/// Flagging two messages inside one transaction the caller owns.
///
/// Same contract as [`relocate`]: the rules pass will call this from inside
/// the sync transaction that inserted the message, so the verb takes a borrow
/// and never commits.
#[test]
fn two_flag_changes_share_one_caller_owned_transaction() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let first = a_message(&connection, inbox, "uid-1");
    let second = a_message(&connection, inbox, "uid-2");
    let rows = read(&connection, &[first, second]);

    let transaction = connection.transaction().expect("open a transaction");
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        true,
        at(9),
    )
    .expect("the first flag change");
    set_flag(
        &transaction,
        account.id,
        &[&rows[1]],
        &Flag::Seen,
        true,
        at(10),
    )
    .expect("the second, in the same transaction");
    transaction.commit().expect("commit");

    for id in [first, second] {
        let row = MessageRepository::new(&connection)
            .get(id)
            .expect("read back")
            .expect("still there");
        assert!(
            row.flags.contains(&Flag::Seen),
            "both flag changes should have landed once the caller committed"
        );
    }
}

/// Setting a flag enqueues `SetFlags`; clearing it enqueues `ClearFlags`.
#[test]
fn clearing_a_flag_enqueues_the_opposite_operation() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    let rows = read(&transaction, &[message]);
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        true,
        at(9),
    )
    .expect("set");
    let rows = read(&transaction, &[message]);
    set_flag(
        &transaction,
        account.id,
        &[&rows[0]],
        &Flag::Seen,
        false,
        at(10),
    )
    .expect("clear");
    transaction.commit().expect("commit");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
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

/// Rows as the verb wants them: it is given the messages, not their ids,
/// because it needs each one's current flags and thread.
fn read(connection: &rusqlite::Connection, ids: &[MessageId]) -> Vec<Message> {
    let messages = MessageRepository::new(connection);
    ids.iter()
        .map(|id| {
            messages
                .get(*id)
                .expect("read a message")
                .expect("the message exists")
        })
        .collect()
}

/// A label is three writes, and this verb owes all three.
///
/// `postio_session::actions::set_label` has always written the join row, the
/// keyword and the queue row together, and the reason is in its own doc
/// comment: the join is what the list and the reader draw, the keyword is
/// what reaches the server, and half of either is a label that is invisible
/// to every other client or one with no name and no colour to draw. Lifting
/// it whole is the point of #1141 -- a rule that wrote only the join would
/// satisfy a test that looks only at `message_labels`.
#[test]
fn set_label_writes_the_join_the_keyword_and_the_queue() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let label = a_label(&connection, account.id, "Invoices");
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    let rows = read(&transaction, &[message]);
    set_label(&transaction, account.id, &[&rows[0]], &label, true, at(9)).expect("set the label");
    transaction.commit().expect("commit");

    assert_eq!(
        LabelRepository::new(&connection)
            .for_message(message)
            .expect("read the labels"),
        vec![label.id],
        "the join row is what the list and the reader draw the label from"
    );
    let row = MessageRepository::new(&connection)
        .get(message)
        .expect("read back")
        .expect("still there");
    assert!(
        row.flags.contains(&Flag::Keyword("Invoices".to_owned())),
        "the label travels to the server as a keyword, so the flag set has \
         to carry it too -- got {:?}",
        row.flags
    );
    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .expect("read the queue");
    assert_eq!(queued.len(), 1, "one operation for one label");
    assert!(
        matches!(
            &queued[0].operation,
            Operation::SetFlags { flags } if flags.contains(&Flag::Keyword("Invoices".to_owned()))
        ),
        "local-first means the queue row as well, carrying the keyword: \
         got {:?}",
        queued[0].operation
    );
}

/// Taking a label off undoes all three, and tells the server so.
#[test]
fn clearing_a_label_detaches_it_and_enqueues_clear_flags() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let label = a_label(&connection, account.id, "Invoices");
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    let rows = read(&transaction, &[message]);
    set_label(&transaction, account.id, &[&rows[0]], &label, true, at(9)).expect("set");
    let rows = read(&transaction, &[message]);
    set_label(&transaction, account.id, &[&rows[0]], &label, false, at(10)).expect("clear");
    transaction.commit().expect("commit");

    assert!(
        LabelRepository::new(&connection)
            .for_message(message)
            .expect("read the labels")
            .is_empty(),
        "clearing has to detach the join row, not only the keyword"
    );
    let row = MessageRepository::new(&connection)
        .get(message)
        .expect("read back")
        .expect("still there");
    assert!(
        !row.flags.contains(&Flag::Keyword("Invoices".to_owned())),
        "and take the keyword back off the row, got {:?}",
        row.flags
    );
    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(23))
        .expect("read the queue");
    assert!(
        matches!(&queued[1].operation, Operation::ClearFlags { .. }),
        "the server is told to clear the keyword, got {:?}",
        queued[1].operation
    );
}

/// Nothing lands if the caller rolls back -- the join row included.
///
/// The same argument as `a_rolled_back_transaction_relocates_nothing`, and
/// worth making separately because this verb writes through three
/// repositories: one of them opening a connection of its own would leave a
/// label behind on a message the pass rolled back.
#[test]
fn a_rolled_back_transaction_labels_nothing() {
    let database = test_support::memory();
    let mut connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let label = a_label(&connection, account.id, "Invoices");
    let message = a_message(&connection, inbox, "uid-9");

    let transaction = connection.transaction().expect("open a transaction");
    let rows = read(&transaction, &[message]);
    set_label(&transaction, account.id, &[&rows[0]], &label, true, at(9)).expect("set the label");
    drop(transaction);

    assert!(
        LabelRepository::new(&connection)
            .for_message(message)
            .expect("read the labels")
            .is_empty(),
        "a rolled-back transaction must leave no label on the message"
    );
    assert!(
        OperationQueueRepository::new(&connection)
            .pending(account.id, at(23))
            .expect("read the queue")
            .is_empty(),
        "and must enqueue nothing"
    );
}

/// A label the account owns, created the way the picker creates one.
fn a_label(connection: &rusqlite::Connection, account: AccountId, name: &str) -> Label {
    let mut label = Label::new(account, name);
    LabelRepository::new(connection)
        .create(&mut label)
        .expect("create a label");
    label
}
