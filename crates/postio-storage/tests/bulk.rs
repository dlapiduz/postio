//! Whole-mailbox actions: the predicate that must never become a list of ids.
//!
//! Triage on an 81,717-message account is what these exist for. `Ctrl+A` then
//! `a` has to reach SQLite as a query — one `UPDATE` and one `INSERT ...
//! SELECT` — because the alternative is reading a mailbox into memory, which
//! docs/PRODUCT.md §18 forbids and the 16 ms interaction budget would not survive.
//!
//! The interesting assertions here are therefore about *statement counts and
//! shapes*, not only about outcomes: a version of this that enumerated the
//! rows first would pass every outcome assertion and still be the bug.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;

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

/// `count` messages in `mailbox`, inserted straight into the table: these
/// tests are about the bulk statements, not about message construction.
fn fill(connection: &Connection, mailbox: MailboxId, count: usize) -> Vec<MessageId> {
    (0..count)
        .map(|index| {
            connection
                .execute(
                    "INSERT INTO messages (account_id, mailbox_id, received_at)
                     SELECT account_id, id, ?2 FROM mailboxes WHERE id = ?1",
                    [mailbox.get(), index as i64],
                )
                .expect("insert a message");
            MessageId::new(connection.last_insert_rowid())
        })
        .collect()
}

fn mailbox_of(connection: &Connection, message: MessageId) -> MailboxId {
    connection
        .query_row(
            "SELECT mailbox_id FROM messages WHERE id = ?1",
            [message.get()],
            |row| row.get::<_, i64>(0).map(MailboxId::new),
        )
        .expect("the message is still there")
}

fn queued(connection: &Connection, account: &Account) -> Vec<QueuedOperation> {
    OperationQueueRepository::new(connection)
        .pending(account.id, at(23))
        .expect("a read")
}

struct World {
    account: Account,
    inbox: MailboxId,
    archive: MailboxId,
}

fn world(connection: &Connection) -> World {
    let (account, inbox) = test_support::account_with_inbox(connection);
    let archive = test_support::mailbox(connection, &account, "Archive").id;
    World {
        account,
        inbox,
        archive,
    }
}

// ── The write ────────────────────────────────────────────────────────────

#[test]
fn moving_a_whole_mailbox_is_one_statement_over_the_index() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 40);

    let moved = MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .expect("a bulk move");

    assert_eq!(moved, 40);
    for message in &messages {
        assert_eq!(mailbox_of(&connection, *message), world.archive);
    }
}

#[test]
fn the_rows_taken_back_out_of_the_selection_stay_put() {
    // `Ctrl+A` then un-ticking two rows. The exceptions are built by clicking,
    // so naming them is affordable in a way naming the other 81,715 is not.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 10);
    let kept = vec![messages[3], messages[7]];

    let moved = MessageRepository::new(&connection)
        .move_set(
            &MessageSet::InMailbox {
                mailbox: world.inbox,
                except: kept.clone(),
            },
            world.archive,
        )
        .expect("a bulk move");

    assert_eq!(moved, 8);
    for message in &kept {
        assert_eq!(
            mailbox_of(&connection, *message),
            world.inbox,
            "a deselected row is not part of the selection"
        );
    }
}

#[test]
fn a_row_hidden_pending_a_remote_delete_is_not_part_of_everything() {
    // The set means "what the list is showing", and the list filters these
    // out. Sweeping them along would resurrect a delete the user already made.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 5);
    MessageRepository::new(&connection)
        .set_deleted_locally(&[messages[0]], true)
        .expect("hide it");

    let moved = MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .expect("a bulk move");

    assert_eq!(moved, 4);
    assert_eq!(mailbox_of(&connection, messages[0]), world.inbox);
}

#[test]
fn a_bulk_move_clears_the_server_identity_the_way_a_single_one_does() {
    // A UID belongs to the mailbox that issued it; keeping it would make the
    // next resync match the wrong message.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 3);
    connection
        .execute(
            "UPDATE messages SET uid = id, uid_validity = 7, mod_seq = 9",
            [],
        )
        .expect("give them a server identity");

    MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .expect("a bulk move");

    let (uid, validity, mod_seq): (Option<i64>, Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT uid, uid_validity, mod_seq FROM messages WHERE id = ?1",
            [messages[0].get()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("a read");
    assert_eq!((uid, validity, mod_seq), (None, None, None));
}

#[test]
fn counting_a_set_never_reads_its_rows() {
    // The toast needs a number and nothing else does. `count(*)` over the
    // mailbox index is the whole of what a bulk action is allowed to know.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 12);

    let messages_repository = MessageRepository::new(&connection);
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::in_mailbox(world.inbox))
            .expect("a count"),
        12
    );
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::InMailbox {
                mailbox: world.inbox,
                except: vec![messages[0]],
            })
            .expect("a count"),
        11
    );
    assert_eq!(
        messages_repository
            .count_set(&MessageSet::in_mailbox(world.archive))
            .expect("a count"),
        0,
        "an empty mailbox is a count, not an error"
    );
}

// ── The queue ────────────────────────────────────────────────────────────

#[test]
fn a_bulk_enqueue_writes_one_row_per_message_naming_each_one() {
    // One row per message, not one row for the mailbox: the drainer needs the
    // UID of a *specific* message, and a row that said "everything in INBOX"
    // would be resolved later, sweeping up mail that arrived in between.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 6);
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
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");

    let rows = queued(&connection, &world.account);
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
            row.is_undoable(),
            "the inverse is decided at enqueue time here as it is anywhere else"
        );
    }
    assert_eq!(range.first, rows[0].id);
    assert_eq!(range.last, rows[5].id);
}

#[test]
fn the_run_a_bulk_enqueue_returns_excludes_what_was_already_queued() {
    // `first` comes from the highest id present *before* the statement, so an
    // earlier action's row cannot be swept into this action's undo.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 4);
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
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");

    assert!(
        range.first.get() > earlier.id.get(),
        "the run starts after the flag that was already queued"
    );
    assert_eq!(
        MessageRepository::new(&connection)
            .count_set(&MessageSet::Queued(range))
            .expect("a count"),
        4,
        "and it names exactly the four the bulk action touched"
    );
}

#[test]
fn a_bulk_enqueue_over_an_empty_mailbox_writes_nothing_and_says_so() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);

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
        .expect("a bulk enqueue");

    assert_eq!(range, None, "nothing to do is not a failure");
    assert!(queued(&connection, &world.account).is_empty());
}

#[test]
fn a_bulk_enqueue_marks_its_messages_as_having_something_pending() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 3);

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
        .expect("a bulk enqueue");

    assert!(
        OperationQueueRepository::new(&connection)
            .has_pending(OperationTarget::Message(messages[1]))
            .expect("a read")
    );
}

// ── The run as undo's handle ─────────────────────────────────────────────

#[test]
fn a_queued_run_names_the_messages_a_bulk_action_moved_and_nothing_else() {
    // This is what makes one `u` take back 81,717 messages without naming
    // them: the queue already numbered them, so two integers are the set.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 5);
    let untouched = fill(&connection, world.archive, 2);

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
        .expect("a bulk enqueue")
        .expect("the mailbox was not empty");
    MessageRepository::new(&connection)
        .move_set(&MessageSet::in_mailbox(world.inbox), world.archive)
        .expect("a bulk move");

    // Now take it back, naming the rows only by the run that moved them.
    let returned = MessageRepository::new(&connection)
        .move_set(&MessageSet::Queued(range), world.inbox)
        .expect("a bulk move back");

    assert_eq!(returned, 5);
    for message in &messages {
        assert_eq!(mailbox_of(&connection, *message), world.inbox);
    }
    for message in &untouched {
        assert_eq!(
            mailbox_of(&connection, *message),
            world.archive,
            "undo puts back what the action moved, not everything in the folder \
             it moved things into"
        );
    }
}

#[test]
fn an_empty_run_names_nothing() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    fill(&connection, world.inbox, 3);

    let empty = OperationRange::new(
        postio_model::OperationId::new(9),
        postio_model::OperationId::new(8),
    );

    assert!(empty.is_empty());
    assert_eq!(
        MessageRepository::new(&connection)
            .count_set(&MessageSet::Queued(empty))
            .expect("a count"),
        0
    );
}

// ── Flags ────────────────────────────────────────────────────────────────
//
// A bulk flag write is not the same statement a bulk move is. `\Seen` and
// `\Flagged` live in a text column *and* in a denormalised boolean beside it,
// so the write has to keep the two agreeing without reading a row — and the
// toggle has to know whether the rows already agree, which is a count rather
// than a scan.

/// The `flags` text a row is holding, straight out of the column.
fn flag_text(connection: &Connection, message: MessageId) -> String {
    connection
        .query_row(
            "SELECT flags FROM messages WHERE id = ?1",
            [message.get()],
            |row| row.get(0),
        )
        .expect("the message is still there")
}

fn boolean(connection: &Connection, message: MessageId, column: &str) -> bool {
    connection
        .query_row(
            &format!("SELECT {column} FROM messages WHERE id = ?1"),
            [message.get()],
            |row| row.get::<_, i64>(0).map(|value| value != 0),
        )
        .expect("the message is still there")
}

/// Puts `text` in the flags column and the booleans that shadow it, the way a
/// repository write would have left them.
fn wearing(connection: &Connection, message: MessageId, text: &str) {
    let has = |flag: &str| text.split_whitespace().any(|word| word == flag);
    connection
        .execute(
            "UPDATE messages SET flags = ?2, seen = ?3, flagged = ?4, answered = ?5,
                                 draft = ?6, deleted = ?7
              WHERE id = ?1",
            rusqlite::params![
                message.get(),
                text,
                has("\\Seen"),
                has("\\Flagged"),
                has("\\Answered"),
                has("\\Draft"),
                has("\\Deleted"),
            ],
        )
        .expect("dress the message");
}

#[test]
fn marking_a_whole_mailbox_read_is_one_statement_over_the_index() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 20);
    let elsewhere = fill(&connection, world.archive, 3);

    let changed = MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .expect("a bulk flag write");

    assert_eq!(changed, 20);
    for message in &messages {
        assert!(boolean(&connection, *message, "seen"));
        assert_eq!(flag_text(&connection, *message), "\\Seen");
    }
    for message in &elsewhere {
        assert!(
            !boolean(&connection, *message, "seen"),
            "the predicate is the mailbox, not the account"
        );
    }
}

#[test]
fn a_bulk_flag_write_keeps_the_text_and_its_booleans_agreeing() {
    // The column is denormalised *from* the text — the list filters on the
    // boolean and the sync engine sends the text, so a write that moved one
    // and not the other would show one thing and tell the server another.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let message = fill(&connection, world.inbox, 1)[0];
    wearing(&connection, message, "\\Answered $Work");

    MessageRepository::new(&connection)
        .set_flag_on_set(
            &MessageSet::in_mailbox(world.inbox),
            ColumnFlag::Flagged,
            true,
        )
        .expect("a bulk flag write");

    assert!(boolean(&connection, message, "flagged"));
    assert_eq!(
        flag_text(&connection, message),
        "\\Answered \\Flagged $Work",
        "canonical spellings in FlagSet order, which is what the schema promises"
    );
}

#[test]
fn clearing_a_flag_in_bulk_leaves_every_other_flag_alone() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let message = fill(&connection, world.inbox, 1)[0];
    wearing(&connection, message, "\\Seen \\Answered \\Flagged $Work");

    MessageRepository::new(&connection)
        .set_flag_on_set(
            &MessageSet::in_mailbox(world.inbox),
            ColumnFlag::Seen,
            false,
        )
        .expect("a bulk flag write");

    assert!(!boolean(&connection, message, "seen"));
    assert_eq!(
        flag_text(&connection, message),
        "\\Answered \\Flagged $Work"
    );
    assert!(
        boolean(&connection, message, "answered"),
        "marking unread is not a reset"
    );
}

#[test]
fn a_bulk_flag_write_marks_its_rows_as_ahead_of_the_server() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let message = fill(&connection, world.inbox, 1)[0];

    MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .expect("a bulk flag write");

    assert!(
        boolean(&connection, message, "flags_dirty"),
        "a local flag change has to be pushed, exactly as `set_flags` records"
    );
}

#[test]
fn the_rows_a_flag_write_would_change_are_a_set_of_their_own() {
    // The rows that *disagree* with the write are the ones the queue must
    // carry, or undo would take back rows the action never touched. Asking
    // which they are stays a count, not a read.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 6);
    for message in &messages[..4] {
        wearing(&connection, *message, "\\Seen");
    }

    let unread = MessageSet::WithFlag {
        set: Box::new(MessageSet::in_mailbox(world.inbox)),
        flag: ColumnFlag::Seen,
        present: false,
    };
    let repository = MessageRepository::new(&connection);

    assert_eq!(repository.count_set(&unread).expect("a count"), 2);
    assert_eq!(
        repository
            .set_flag_on_set(&unread, ColumnFlag::Seen, true)
            .expect("a bulk flag write"),
        2,
        "the four that already agreed are not written again"
    );
    assert_eq!(repository.count_set(&unread).expect("a count"), 0);
}

#[test]
fn a_flag_set_still_honours_the_rows_taken_back_out_of_the_selection() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 5);

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
            .expect("a bulk flag write"),
        4
    );
    assert!(!boolean(&connection, messages[1], "seen"));
}

#[test]
fn a_flag_set_composes_with_the_queued_run_undo_names() {
    // The undo half of a bulk flag: the run of queue rows the action wrote is
    // the set, narrowed to the rows that still carry what it put on them.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    let messages = fill(&connection, world.inbox, 4);
    let untouched = fill(&connection, world.inbox, 2);
    for message in &untouched {
        wearing(&connection, *message, "\\Flagged");
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
        .expect("a bulk enqueue")
        .expect("four rows disagreed");
    let repository = MessageRepository::new(&connection);
    repository
        .set_flag_on_set(&unflagged, ColumnFlag::Flagged, true)
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
        .expect("a bulk flag write back");

    assert_eq!(taken_back, 4);
    for message in &messages {
        assert!(!boolean(&connection, *message, "flagged"));
    }
    for message in &untouched {
        assert!(
            boolean(&connection, *message, "flagged"),
            "undo takes back what the action flagged, not what was flagged already"
        );
    }
}

#[test]
fn a_bulk_flag_write_moves_the_cached_mailbox_counts() {
    // The sidebar reads these rather than counting rows, so a bulk write that
    // did not move them would show an unread badge over a folder the user has
    // just marked read.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let world = world(&connection);
    fill(&connection, world.inbox, 7);

    MessageRepository::new(&connection)
        .set_flag_on_set(&MessageSet::in_mailbox(world.inbox), ColumnFlag::Seen, true)
        .expect("a bulk flag write");

    let unread: i64 = connection
        .query_row(
            "SELECT unread_count FROM mailboxes WHERE id = ?1",
            [world.inbox.get()],
            |row| row.get(0),
        )
        .expect("a count");
    assert_eq!(unread, 0);
}

#[test]
fn the_flags_with_columns_are_the_five_that_sort_first() {
    // The guard under `set_flag_on_set`. It rebuilds the flags text as "the
    // five system flags, in column order, then the keywords", which is only
    // the same string `FlagSet` would have produced while those five remain
    // the five lowest-ranked persistable flags. Add a sixth system flag ahead
    // of them in `Flag::rank` and every bulk flag write starts writing the
    // text out of order — silently, because nothing reads it back in order.
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
