use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;
use postio_storage::sql::bind;

use postio_model::{
    Account, MailboxId, MessageId, Operation, OperationRange, OperationState, OperationTarget,
};
use postio_storage::repository::{
    ColumnFlag, MessageRepository, MessageSet, OperationQueueRepository, QueuedOperation,
};
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 24, hour, 0, 0).unwrap()
}

async fn fill(connection: &Connection, mailbox: MailboxId, count: usize) -> Vec<MessageId> {
    let mut collected = Vec::new();
    for index in 0..count {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at)
                     SELECT account_id, id, ?2, ?2 FROM mailboxes WHERE id = ?1",
                [mailbox.get(), index as i64],
            )
            .await
            .expect("insert a message");
        collected.push(MessageId::new(connection.last_insert_rowid()));
    }
    collected
}

async fn mailbox_of(connection: &Connection, message: MessageId) -> MailboxId {
    postio_storage::sql::one(
        connection,
        "SELECT mailbox_id FROM messages WHERE id = ?1",
        bind![message.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0).map(MailboxId::new),
    )
    .await
    .expect("the message is still there")
}

async fn queued(connection: &Connection, account: &Account) -> Vec<QueuedOperation> {
    OperationQueueRepository::new(connection)
        .pending(account.id, at(23))
        .await
        .expect("a read")
}

struct World {
    account: Account,
    inbox: MailboxId,
    archive: MailboxId,
}

async fn world(connection: &Connection) -> World {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let archive = test_support::mailbox(connection, &account, "Archive")
        .await
        .id;
    World {
        account,
        inbox,
        archive,
    }
}

#[tokio::test]
async fn moving_a_whole_mailbox_is_one_statement_over_the_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 40).await;

    let moved = MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .await
        .expect("a bulk move");

    assert_eq!(moved, 40);
    for message in &messages {
        assert_eq!(mailbox_of(&connection, *message).await, world.archive);
    }
}

#[tokio::test]
async fn the_rows_taken_back_out_of_the_selection_stay_put() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 10).await;
    let kept = vec![messages[3], messages[7]];

    let moved = MessageRepository::new(&connection)
        .move_set(
            &MessageSet::InMailbox {
                mailbox: world.inbox,
                except: kept.clone(),
            },
            world.archive,
        )
        .await
        .expect("a bulk move");

    assert_eq!(moved, 8);
    for message in &kept {
        assert_eq!(
            mailbox_of(&connection, *message).await,
            world.inbox,
            "a deselected row is not part of the selection"
        );
    }
}

/// `fill` leaves `thread_id` NULL, the other case the predicate must handle (the `IS NULL` arm in
/// `MessageSet::predicate`).
async fn fill_thread(
    connection: &Connection,
    account: &Account,
    mailbox: MailboxId,
    count: usize,
) -> (i64, Vec<MessageId>) {
    connection
        .execute(
            "INSERT INTO threads (account_id, subject) VALUES (?1, 'A conversation')",
            [account.id.get()],
        )
        .await
        .expect("insert a thread");
    let thread = connection.last_insert_rowid();
    let mut messages = Vec::new();
    for index in 0..count {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, thread_id)
                     SELECT account_id, id, ?2, ?2, ?3 FROM mailboxes WHERE id = ?1",
                [mailbox.get(), index as i64, thread],
            )
            .await
            .expect("insert a message");
        messages.push(MessageId::new(connection.last_insert_rowid()));
    }
    (thread, messages)
}

#[tokio::test]
async fn deselecting_a_thread_row_excepts_the_whole_conversation() {
    // The row taken back out of `Ctrl+A` is a conversation, identified by its newest message (ADR
    // 0015
    // Q3, #468); excepting only that id would archive all of it but the newest.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;

    let (_, kept) = fill_thread(&connection, &world.account, world.inbox, 4).await;
    let (_, swept) = fill_thread(&connection, &world.account, world.inbox, 3).await;
    let representative = *kept.last().expect("a conversation has messages");

    let moved = MessageRepository::new(&connection)
        .move_set(
            &MessageSet::InMailbox {
                mailbox: world.inbox,
                except: vec![representative],
            },
            world.archive,
        )
        .await
        .expect("a bulk move");

    assert_eq!(
        moved,
        swept.len(),
        "the other conversation is what was left to move"
    );
    for message in &kept {
        assert_eq!(
            mailbox_of(&connection, *message).await,
            world.inbox,
            "a message of the deselected conversation was archived anyway: \
             the exception named the row, and the row is the conversation"
        );
    }
    for message in &swept {
        assert_eq!(
            mailbox_of(&connection, *message).await,
            world.archive,
            "excepting one conversation must not spare another"
        );
    }
}

#[tokio::test]
async fn an_unthreaded_message_is_still_excepted_by_its_own_id() {
    // The `IS NULL` arm: `NOT IN` against a set containing NULL is NULL, so without the guard
    // unthreaded messages would drop out of the set and nothing would move.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 5).await;

    let moved = MessageRepository::new(&connection)
        .move_set(
            &MessageSet::InMailbox {
                mailbox: world.inbox,
                except: vec![messages[2]],
            },
            world.archive,
        )
        .await
        .expect("a bulk move");

    assert_eq!(
        moved, 4,
        "the other four are unthreaded and still in the set"
    );
    assert_eq!(mailbox_of(&connection, messages[2]).await, world.inbox);
}

#[tokio::test]
async fn a_row_hidden_pending_a_remote_delete_is_not_part_of_everything() {
    // Sweeping already-deleted rows along would resurrect a delete the user made.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 5).await;
    MessageRepository::new(&connection)
        .set_deleted_locally(&[messages[0]], true)
        .await
        .expect("hide it");

    let moved = MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .await
        .expect("a bulk move");

    assert_eq!(moved, 4);
    assert_eq!(mailbox_of(&connection, messages[0]).await, world.inbox);
}

#[tokio::test]
async fn a_bulk_move_clears_the_server_identity_the_way_a_single_one_does() {
    // A UID belongs to the mailbox that issued it; keeping it would make the
    // next resync match the wrong message.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 3).await;
    connection
        .execute(
            "UPDATE messages SET uid = id, uid_validity = 7, mod_seq = 9",
            (),
        )
        .await
        .expect("give them a server identity");

    MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .await
        .expect("a bulk move");

    let (uid, validity, mod_seq): (Option<i64>, Option<i64>, Option<i64>) =
        postio_storage::sql::one(
            &connection,
            "SELECT uid, uid_validity, mod_seq FROM messages WHERE id = ?1",
            bind![messages[0].get()],
            |row| {
                Ok((
                    postio_storage::sql::RowExt::col(row, 0)?,
                    postio_storage::sql::RowExt::col(row, 1)?,
                    postio_storage::sql::RowExt::col(row, 2)?,
                ))
            },
        )
        .await
        .expect("a read");
    assert_eq!((uid, validity, mod_seq), (None, None, None));
}

#[tokio::test]
async fn counting_a_set_never_reads_its_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 12).await;

    let messages_repository = MessageRepository::new(&connection);
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::in_mailbox(world.inbox))
            .await
            .expect("a count"),
        12
    );
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::InMailbox {
                mailbox: world.inbox,
                except: vec![messages[0]],
            })
            .await
            .expect("a count"),
        11
    );
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::in_mailbox(world.archive))
            .await
            .expect("a count"),
        0,
        "an empty mailbox is a count, not an error"
    );
}

#[tokio::test]
async fn a_bulk_enqueue_writes_one_row_per_message_naming_each_one() {
    // One row per message: a row saying "everything in INBOX" would be resolved later and sweep up
    // mail that arrived in between.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 6).await;
    let archive = Operation::Move {
        from: world.inbox,
        to: world.archive,
    };

    let range = OperationQueueRepository::new(&connection)
        .enqueue_set(
            world.account.id,
            &MessageSet::in_mailbox(world.inbox),
            &archive,
            at(9),
        )
        .await
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");

    let rows = queued(&connection, &world.account).await;
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows.iter()
            .map(|row| row.target)
            .collect::<Vec<OperationTarget>>(),
        messages
            .iter()
            .map(|id| OperationTarget::Message(*id))
            .collect::<Vec<_>>(),
        "every row names the message whose UID the drainer will need"
    );
    for row in &rows {
        assert_eq!(row.operation, archive);
        assert_eq!(row.state, OperationState::Pending);
        assert_eq!(row.mailbox_id, Some(world.inbox));
        assert!(
            row.inverse.is_some(),
            "the inverse is decided at enqueue time here as it is anywhere else"
        );
    }
    assert_eq!(range.first, rows[0].id);
    assert_eq!(range.last, rows[5].id);
}

#[tokio::test]
async fn the_run_a_bulk_enqueue_returns_excludes_what_was_already_queued() {
    // `first` comes from the highest id present *before* the statement, so an
    // earlier action's row cannot be swept into this action's undo.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 4).await;
    let queue = OperationQueueRepository::new(&connection);
    let earlier = queue
        .enqueue(
            world.account.id,
            OperationTarget::Message(messages[0]),
            &Operation::SetFlags {
                flags: std::iter::once(postio_model::Flag::Flagged).collect(),
            },
            at(8),
        )
        .await
        .expect("an earlier action");

    let range = queue
        .enqueue_set(
            world.account.id,
            &MessageSet::in_mailbox(world.inbox),
            &Operation::Move {
                from: world.inbox,
                to: world.archive,
            },
            at(9),
        )
        .await
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");

    assert!(
        range.first.get() > earlier.id.get(),
        "the run starts after the flag that was already queued"
    );
    assert_eq!(
        MessageRepository::new(&connection)
            .count_set(&MessageSet::Queued(range))
            .await
            .expect("a count"),
        4,
        "and it names exactly the four the bulk action touched"
    );
}

#[tokio::test]
async fn a_bulk_enqueue_over_an_empty_mailbox_writes_nothing_and_says_so() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;

    let range = OperationQueueRepository::new(&connection)
        .enqueue_set(
            world.account.id,
            &MessageSet::in_mailbox(world.inbox),
            &Operation::Move {
                from: world.inbox,
                to: world.archive,
            },
            at(9),
        )
        .await
        .expect("a bulk enqueue");

    assert_eq!(range, None, "nothing to do is not a failure");
    assert!(queued(&connection, &world.account).await.is_empty());
}

#[tokio::test]
async fn a_bulk_enqueue_marks_its_messages_as_having_something_pending() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 3).await;

    OperationQueueRepository::new(&connection)
        .enqueue_set(
            world.account.id,
            &MessageSet::in_mailbox(world.inbox),
            &Operation::Move {
                from: world.inbox,
                to: world.archive,
            },
            at(9),
        )
        .await
        .expect("a bulk enqueue");

    assert!(
        OperationQueueRepository::new(&connection)
            .has_pending(OperationTarget::Message(messages[1]))
            .await
            .expect("a read")
    );
}

#[tokio::test]
async fn a_queued_run_names_the_messages_a_bulk_action_moved_and_nothing_else() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 5).await;
    let untouched = fill(&connection, world.archive, 2).await;

    let queue = OperationQueueRepository::new(&connection);
    let range = queue
        .enqueue_set(
            world.account.id,
            &MessageSet::in_mailbox(world.inbox),
            &Operation::Move {
                from: world.inbox,
                to: world.archive,
            },
            at(9),
        )
        .await
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");
    MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .await
        .expect("a bulk move");

    let returned = MessageRepository::new(&connection)
        .move_set(&MessageSet::Queued(range), world.inbox)
        .await
        .expect("a bulk move back");

    assert_eq!(returned, 5);
    for message in &messages {
        assert_eq!(mailbox_of(&connection, *message).await, world.inbox);
    }
    for message in &untouched {
        assert_eq!(
            mailbox_of(&connection, *message).await,
            world.archive,
            "undo puts back what the action moved, not everything in the folder \
             it moved things into"
        );
    }
}

#[tokio::test]
async fn an_empty_run_names_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    fill(&connection, world.inbox, 3).await;

    let empty = OperationRange::new(
        postio_model::OperationId::new(9),
        postio_model::OperationId::new(8),
    );

    assert!(empty.is_empty());
    assert_eq!(
        MessageRepository::new(&connection)
            .count_set(&MessageSet::Queued(empty))
            .await
            .expect("a count"),
        0
    );
}

async fn flag_text(connection: &Connection, message: MessageId) -> String {
    postio_storage::sql::one(
        connection,
        "SELECT flags FROM messages WHERE id = ?1",
        bind![message.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the message is still there")
}

async fn boolean(connection: &Connection, message: MessageId, column: &str) -> bool {
    postio_storage::sql::one(
        connection,
        &format!("SELECT {column} FROM messages WHERE id = ?1"),
        bind![message.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0).map(|value| value != 0),
    )
    .await
    .expect("the message is still there")
}

async fn wearing(connection: &Connection, message: MessageId, text: &str) {
    let has = |flag: &str| text.split_whitespace().any(|word| word == flag);
    connection
        .execute(
            "UPDATE messages SET flags = ?2, seen = ?3, flagged = ?4, answered = ?5,
                                 draft = ?6, deleted = ?7
              WHERE id = ?1",
            bind![
                message.get(),
                text,
                has("\\Seen"),
                has("\\Flagged"),
                has("\\Answered"),
                has("\\Draft"),
                has("\\Deleted"),
            ],
        )
        .await
        .expect("dress the message");
}

#[tokio::test]
async fn marking_a_whole_mailbox_read_is_one_statement_over_the_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 20).await;
    let elsewhere = fill(&connection, world.archive, 3).await;

    let changed = MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .await
        .expect("a bulk flag write");

    assert_eq!(changed, 20);
    for message in &messages {
        assert!(boolean(&connection, *message, "seen").await);
        assert_eq!(flag_text(&connection, *message).await, "\\Seen");
    }
    for message in &elsewhere {
        assert!(
            !boolean(&connection, *message, "seen").await,
            "the predicate is the mailbox, not the account"
        );
    }
}

#[tokio::test]
async fn a_bulk_flag_write_keeps_the_text_and_its_booleans_agreeing() {
    // The boolean is denormalised from the text: the list filters on one and sync sends the other.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let message = fill(&connection, world.inbox, 1).await[0];
    wearing(&connection, message, "\\Answered $Work").await;

    MessageRepository::new(&connection)
        .set_flag_on_set(
            &MessageSet::in_mailbox(world.inbox),
            ColumnFlag::Flagged,
            true,
        )
        .await
        .expect("a bulk flag write");

    assert!(boolean(&connection, message, "flagged").await);
    assert_eq!(
        flag_text(&connection, message).await,
        "\\Answered \\Flagged $Work",
        "canonical spellings in FlagSet order, which is what the schema promises"
    );
}

#[tokio::test]
async fn clearing_a_flag_in_bulk_leaves_every_other_flag_alone() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let message = fill(&connection, world.inbox, 1).await[0];
    wearing(&connection, message, "\\Seen \\Answered \\Flagged $Work").await;

    MessageRepository::new(&connection)
        .set_flag_on_set(
            &MessageSet::in_mailbox(world.inbox),
            ColumnFlag::Seen,
            false,
        )
        .await
        .expect("a bulk flag write");

    assert!(!boolean(&connection, message, "seen").await);
    assert_eq!(
        flag_text(&connection, message).await,
        "\\Answered \\Flagged $Work"
    );
    assert!(
        boolean(&connection, message, "answered").await,
        "marking unread is not a reset"
    );
}

#[tokio::test]
async fn a_bulk_flag_write_marks_its_rows_as_ahead_of_the_server() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let message = fill(&connection, world.inbox, 1).await[0];

    MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .await
        .expect("a bulk flag write");

    assert!(
        boolean(&connection, message, "flags_dirty").await,
        "a local flag change has to be pushed, exactly as `set_flags` records"
    );
}

#[tokio::test]
async fn the_rows_a_flag_write_would_change_are_a_set_of_their_own() {
    // The queue must carry the rows that disagree with the write, or undo takes back rows the
    // action
    // never touched; asking which they are stays a count.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 6).await;
    for message in &messages[..4] {
        wearing(&connection, *message, "\\Seen").await;
    }

    let unread = MessageSet::WithFlag {
        set: Box::new(MessageSet::in_mailbox(world.inbox)),
        flag: ColumnFlag::Seen,
        present: false,
    };
    let repository = MessageRepository::new(&connection);

    assert_eq!(repository.count_set(&unread).await.expect("a count"), 2);
    assert_eq!(
        repository
            .set_flag_on_set(&unread, ColumnFlag::Seen, true)
            .await
            .expect("a bulk flag write"),
        2,
        "the four that already agreed are not written again"
    );
    assert_eq!(repository.count_set(&unread).await.expect("a count"), 0);
}

#[tokio::test]
async fn a_flag_set_still_honours_the_rows_taken_back_out_of_the_selection() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 5).await;

    let set = MessageSet::WithFlag {
        set: Box::new(MessageSet::InMailbox {
            mailbox: world.inbox,
            except: vec![messages[1]],
        }),
        flag: ColumnFlag::Seen,
        present: false,
    };

    assert_eq!(
        MessageRepository::new(&connection)
            .set_flag_on_set(&set, ColumnFlag::Seen, true)
            .await
            .expect("a bulk flag write"),
        4
    );
    assert!(!boolean(&connection, messages[1], "seen").await);
}

#[tokio::test]
async fn a_flag_set_composes_with_the_queued_run_undo_names() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 4).await;
    let untouched = fill(&connection, world.inbox, 2).await;
    for message in &untouched {
        wearing(&connection, *message, "\\Flagged").await;
    }

    let unflagged = MessageSet::WithFlag {
        set: Box::new(MessageSet::in_mailbox(world.inbox)),
        flag: ColumnFlag::Flagged,
        present: false,
    };
    let range = OperationQueueRepository::new(&connection)
        .enqueue_set(
            world.account.id,
            &unflagged,
            &Operation::SetFlags {
                flags: [postio_model::Flag::Flagged].into_iter().collect(),
            },
            at(9),
        )
        .await
        .expect("a bulk enqueue")
        .expect("four rows disagreed");
    let repository = MessageRepository::new(&connection);
    repository
        .set_flag_on_set(&unflagged, ColumnFlag::Flagged, true)
        .await
        .expect("a bulk flag write");

    let taken_back = repository
        .set_flag_on_set(
            &MessageSet::WithFlag {
                set: Box::new(MessageSet::Queued(range)),
                flag: ColumnFlag::Flagged,
                present: true,
            },
            ColumnFlag::Flagged,
            false,
        )
        .await
        .expect("a bulk flag write back");

    assert_eq!(taken_back, 4);
    for message in &messages {
        assert!(!boolean(&connection, *message, "flagged").await);
    }
    for message in &untouched {
        assert!(
            boolean(&connection, *message, "flagged").await,
            "undo takes back what the action flagged, not what was flagged already"
        );
    }
}

#[tokio::test]
async fn a_bulk_flag_write_moves_the_cached_mailbox_counts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    fill(&connection, world.inbox, 7).await;

    MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .await
        .expect("a bulk flag write");

    let unread: i64 = postio_storage::sql::one(
        &connection,
        "SELECT unread_count FROM mailboxes WHERE id = ?1",
        [world.inbox.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("a count");
    assert_eq!(unread, 0);
}

#[test]
fn the_flags_with_columns_are_the_five_that_sort_first() {
    // Guards `set_flag_on_set`, which writes the five system flags in column order, then keywords:
    // equal to `FlagSet`'s text only while those stay the five lowest-ranked in `Flag::rank`.
    let everything: postio_model::FlagSet = [
        postio_model::Flag::parse("Work"),
        postio_model::Flag::NotJunk,
        postio_model::Flag::Junk,
        postio_model::Flag::Forwarded,
        postio_model::Flag::Recent,
        postio_model::Flag::Draft,
        postio_model::Flag::Deleted,
        postio_model::Flag::Flagged,
        postio_model::Flag::Answered,
        postio_model::Flag::Seen,
    ]
    .into_iter()
    .collect();

    let persistable = everything.persistable();
    let spellings: Vec<&str> = persistable.iter().map(postio_model::Flag::as_str).collect();

    assert_eq!(
        &spellings[..5],
        &["\\Seen", "\\Answered", "\\Flagged", "\\Deleted", "\\Draft"],
        "the columns `set_flag_on_set` rebuilds the head from, in that order"
    );
}

async fn flag(connection: &Connection, messages: &[MessageId]) {
    for message in messages {
        connection
            .execute(
                "UPDATE messages SET flagged = 1 WHERE id = ?1",
                [message.get()],
            )
            .await
            .expect("flag a message");
    }
}

#[tokio::test]
async fn the_flagged_set_spans_every_mailbox_in_the_account() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let in_inbox = fill(&connection, world.inbox, 6).await;
    let in_archive = fill(&connection, world.archive, 4).await;

    flag(&connection, &in_inbox[..3]).await;
    flag(&connection, &in_archive[..2]).await;

    let counted = MessageRepository::new(&connection)
        .count_set(&MessageSet::flagged(world.account.id))
        .await
        .expect("a count");

    assert_eq!(
        counted, 5,
        "the set is every flagged message in the account, wherever it is filed"
    );
}

#[tokio::test]
async fn a_deselected_row_comes_out_of_the_flagged_set_too() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let messages = fill(&connection, world.inbox, 5).await;
    flag(&connection, &messages).await;

    let counted = MessageRepository::new(&connection)
        .count_set(&MessageSet::Flagged {
            account: world.account.id,
            except: vec![messages[0], messages[1]],
        })
        .await
        .expect("a count");

    assert_eq!(counted, 3, "clicking two rows off takes them out of it");
}

#[test]
fn a_smart_folder_set_names_no_mailbox() {
    assert_eq!(
        MessageSet::flagged(postio_model::AccountId::new(1)).mailbox(),
        None
    );
}

#[tokio::test]
async fn narrowing_the_flagged_set_to_one_folder_keeps_it_a_predicate() {
    // `Operation::Move` carries one `from` per run, so a move out of a smart folder is grouped by
    // source mailbox, each group still a predicate.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let in_inbox = fill(&connection, world.inbox, 4).await;
    let in_archive = fill(&connection, world.archive, 3).await;
    flag(&connection, &in_inbox).await;
    flag(&connection, &in_archive).await;

    let repository = MessageRepository::new(&connection);
    assert_eq!(
        repository
            .count_set(&MessageSet::flagged(world.account.id).within(world.inbox))
            .await
            .expect("a count"),
        4
    );
    assert_eq!(
        repository
            .count_set(&MessageSet::flagged(world.account.id).within(world.archive))
            .await
            .expect("a count"),
        3
    );
}

#[tokio::test]
async fn the_source_mailboxes_of_a_set_are_answered_without_naming_its_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let world = world(&connection).await;
    let in_inbox = fill(&connection, world.inbox, 500).await;
    let in_archive = fill(&connection, world.archive, 500).await;
    flag(&connection, &in_inbox[..2]).await;
    flag(&connection, &in_archive[..2]).await;

    let mut folders = MessageRepository::new(&connection)
        .mailboxes_of_set(&MessageSet::flagged(world.account.id))
        .await
        .expect("the source folders");
    folders.sort_by_key(|mailbox| mailbox.get());

    let mut expected = vec![world.inbox, world.archive];
    expected.sort_by_key(|mailbox| mailbox.get());
    assert_eq!(folders, expected);
}
