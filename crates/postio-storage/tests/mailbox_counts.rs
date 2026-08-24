//! The cached mailbox counts, and the invariant that they have a writer.
//!
//! `mailboxes.total_count` is not decoration. `postio-runtime`'s store answers
//! the message list's "how many rows are there" from that column rather than
//! by counting, because the list asks with every page and `count(*)` over a
//! folder is linear in its size. The list is a `GListModel`: a total of zero
//! means `GtkListView` asks for no pages at all, so a wrong count here is not
//! a wrong number on screen, it is an empty mailbox.
//!
//! That is what `postio-qhz.7` turned out to be. On a live account with 81,716
//! messages in the store, every `total_count` was 0 and the list drew nothing
//! in every folder — while `select count(*) from messages` returned the real
//! number. `MailboxRepository::recount` existed, was tested, and had one
//! production caller in the whole workspace (the Sent box after a send). The
//! column was derived data with no owner.
//!
//! It has one now: triggers on `messages` maintain it, so every writer keeps
//! it true without knowing it exists. These tests are about that invariant
//! rather than about any one repository method — each goes through a *caller*
//! and reads the cached row back without recounting, because a test that
//! recounted first would pass against the bug.

use chrono::{TimeZone, Utc};
use rusqlite::Connection;

use postio_model::{
    Account, Flag, FlagSet, MailboxId, MailboxRole, Message, MessageId, Uid, UidValidity,
};
use postio_storage::repository::{FlagSource, MailboxRepository, MessageRepository};
use postio_storage::test_support;

/// The cached counts as the sidebar reads them: off the mailbox row, with
/// nothing recounted on the way.
fn cached(connection: &Connection, mailbox: MailboxId) -> (u32, u32, u32) {
    let counts = MailboxRepository::new(connection)
        .counts(mailbox)
        .expect("read the cached counts")
        .expect("the mailbox exists");
    (counts.total, counts.unread, counts.flagged)
}

/// A message with only what the counts care about.
fn a_message(account: &Account, mailbox: MailboxId, uid: u32, flags: &[Flag]) -> Message {
    let at = Utc
        .timestamp_opt(1_770_000_000 + i64::from(uid), 0)
        .unwrap();
    let mut message = Message::new(account.id, mailbox, at);
    message.subject = Some(format!("Message {uid}"));
    message.flags = flags.iter().cloned().collect::<FlagSet>();
    message.server.uid = Some(Uid::new(uid));
    message.server.uid_validity = Some(UidValidity::new(7));
    message
}

/// Writes `count` messages and hands back their ids.
fn write(
    connection: &Connection,
    account: &Account,
    mailbox: MailboxId,
    flags: &[&[Flag]],
) -> Vec<MessageId> {
    let messages = MessageRepository::new(connection);
    flags
        .iter()
        .enumerate()
        .map(|(index, flags)| {
            let mut message = a_message(account, mailbox, index as u32 + 1, flags);
            messages.create(&mut message).expect("write a message")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The counts follow the messages
// ---------------------------------------------------------------------------

#[test]
fn writing_messages_moves_the_counts_without_anyone_recounting() {
    // The failure this is about: a sync writes tens of thousands of rows and
    // the list still believes the folder is empty.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    assert_eq!(cached(&connection, inbox), (0, 0, 0), "a new folder");

    write(
        &connection,
        &account,
        inbox,
        &[&[], &[Flag::Seen], &[Flag::Seen, Flag::Flagged]],
    );

    assert_eq!(
        cached(&connection, inbox),
        (3, 1, 1),
        "three messages, one unread, one flagged — and nothing called recount"
    );
}

#[test]
fn a_batch_upsert_counts_each_row_once() {
    // The sync path. `upsert_batch` inserts what is new and updates what is
    // already there, in one transaction, and a second pass over the same UIDs
    // must not double the folder.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut batch: Vec<Message> = (1..=4)
        .map(|uid| a_message(&account, inbox, uid, &[]))
        .collect();
    messages.upsert_batch(&mut batch).expect("first pass");
    assert_eq!(cached(&connection, inbox), (4, 4, 0));

    // The same UIDs again — an interrupted pass resuming, which is ordinary.
    let mut again: Vec<Message> = (1..=4)
        .map(|uid| a_message(&account, inbox, uid, &[Flag::Seen]))
        .collect();
    messages.upsert_batch(&mut again).expect("second pass");
    assert_eq!(
        cached(&connection, inbox),
        (4, 0, 0),
        "the same four messages, now read — not eight messages"
    );
}

#[test]
fn reading_a_message_moves_the_unread_count() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let ids = write(&connection, &account, inbox, &[&[], &[]]);
    assert_eq!(cached(&connection, inbox), (2, 2, 0));

    MessageRepository::new(&connection)
        .set_flags(
            ids[0],
            &[Flag::Seen].into_iter().collect(),
            FlagSource::Local,
        )
        .expect("mark it read");

    assert_eq!(
        cached(&connection, inbox),
        (2, 1, 0),
        "reading a message does not remove it from the folder"
    );
}

#[test]
fn moving_a_message_moves_its_count_with_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let ids = write(&connection, &account, inbox, &[&[], &[Flag::Flagged]]);
    assert_eq!(cached(&connection, inbox), (2, 2, 1));

    MessageRepository::new(&connection)
        .move_to(&ids[1..], archive)
        .expect("archive it");

    assert_eq!(cached(&connection, inbox), (1, 1, 0), "the folder it left");
    assert_eq!(
        cached(&connection, archive),
        (1, 1, 1),
        "and the one it joined"
    );
}

#[test]
fn a_message_hidden_locally_leaves_the_counts_and_comes_back() {
    // `deleted_locally` is what makes delete feel instant: the row stays and
    // the list stops showing it. The counts have to agree with the list, or
    // the sidebar promises rows the folder will not produce — and undo has to
    // put the number back.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let ids = write(&connection, &account, inbox, &[&[], &[Flag::Flagged]]);
    let messages = MessageRepository::new(&connection);

    messages.set_deleted_locally(&ids[1..], true).expect("hide");
    assert_eq!(cached(&connection, inbox), (1, 1, 0));

    messages
        .set_deleted_locally(&ids[1..], false)
        .expect("undo");
    assert_eq!(cached(&connection, inbox), (2, 2, 1), "undo restores it");
}

#[test]
fn deleting_a_row_takes_it_out_of_the_counts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let ids = write(&connection, &account, inbox, &[&[], &[]]);

    MessageRepository::new(&connection)
        .delete(&ids[..1])
        .expect("expunge it");

    assert_eq!(cached(&connection, inbox), (1, 1, 0));
}

#[test]
fn hiding_a_message_twice_does_not_take_it_out_twice() {
    // The counts are maintained by arithmetic, so a write that sets a column
    // to what it already held is the case that would drift.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let ids = write(&connection, &account, inbox, &[&[], &[]]);
    let messages = MessageRepository::new(&connection);

    messages.set_deleted_locally(&ids[..1], true).expect("hide");
    messages
        .set_deleted_locally(&ids[..1], true)
        .expect("hide it again");

    assert_eq!(cached(&connection, inbox), (1, 1, 0));
}

// ---------------------------------------------------------------------------
// Repairing a store written before the column had a writer
// ---------------------------------------------------------------------------

#[test]
fn a_store_written_without_the_counts_repairs_itself_on_open() {
    // The 81,716-message store from `postio-qhz.7`. Its rows are all there and
    // every cached count is zero, so nothing lists until the column is made
    // true — and it has to happen on open, offline, because a populated local
    // store is populated whether or not a server can be reached.
    let mut connection = Connection::open_in_memory().expect("a database");
    let old = &postio_storage::migrations::all()[..2];
    postio_storage::migrations::migrate_with(&mut connection, old).expect("the old schema");

    connection
        .execute(
            "INSERT INTO accounts (display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES ('Test', 'ada@example.com', 'imap.example.com', 993, 'ada',
                     'smtp.example.com', 587, 'ada', 0)",
            [],
        )
        .expect("an account");
    connection
        .execute(
            "INSERT INTO mailboxes (account_id, name, path, role) VALUES (1, 'INBOX', 'INBOX', 'inbox')",
            [],
        )
        .expect("a folder");
    for uid in 1..=5 {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, seen, flagged)
                 VALUES (1, 1, ?1, ?2, ?3)",
                rusqlite::params![uid, i64::from(uid > 3), i64::from(uid == 1)],
            )
            .expect("a message");
    }
    let inbox = MailboxId::new(1);
    assert_eq!(
        cached(&connection, inbox),
        (0, 0, 0),
        "the state the bug leaves behind"
    );

    postio_storage::migrations::migrate(&mut connection).expect("migrate to head");

    assert_eq!(
        cached(&connection, inbox),
        (5, 3, 1),
        "the store repairs its own counts rather than waiting for a sync"
    );

    // And it keeps them from there on, without a recount.
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at) VALUES (1, 1, 99)",
            [],
        )
        .expect("one more");
    assert_eq!(cached(&connection, inbox), (6, 4, 1));
}

#[test]
fn a_seeded_store_still_agrees_with_a_recount() {
    // The counts now have two writers — the triggers, and `recount` as the
    // repair path. They must not disagree, or which one ran last decides what
    // the sidebar says.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let report = postio_storage::seed::seed_small(&database, 12);
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");

    let before = cached(&connection, inbox.id);
    let recounted = MailboxRepository::new(&connection)
        .recount(inbox.id)
        .expect("recount");

    assert_eq!(
        before,
        (recounted.total, recounted.unread, recounted.flagged),
        "the triggers and the recount have to mean the same thing"
    );
}
