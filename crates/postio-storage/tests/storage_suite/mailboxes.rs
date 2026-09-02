//! Mailboxes: CRUD, special-use roles, sync state, and the sidebar's counts.

use chrono::{TimeZone, Utc};
use rusqlite::Connection;

use postio_model::{
    Account, AccountId, EmailAddress, Generation, Mailbox, MailboxCounts, MailboxId, MailboxRole,
    ModSeq, Uid,
};
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_storage::test_support;

fn seeded_account(connection: &Connection) -> AccountId {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "test@example.com"),
    );
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("create an account")
}

/// Inserts a message straight into the table. The message repository is a
/// separate bead; the counts have to be right before it exists.
fn insert_message(connection: &Connection, mailbox: MailboxId, flags: &str) {
    let seen = i64::from(flags.contains("\\Seen"));
    let flagged = i64::from(flags.contains("\\Flagged"));
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, flags, seen, flagged)
             SELECT account_id, id, 0, ?2, ?3, ?4 FROM mailboxes WHERE id = ?1",
            rusqlite::params![mailbox.get(), flags, seen, flagged],
        )
        .expect("insert a message");
}

// ---------------------------------------------------------------------------
// Create and read
// ---------------------------------------------------------------------------

#[test]
fn a_mailbox_round_trips_through_the_database() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    mailbox.generation = Some(Generation::new(1_707_000_000));
    mailbox.uid_next = Some(Uid::new(4_412));
    mailbox.highest_mod_seq = Some(ModSeq::new(90_210));
    mailbox.last_synced_at = Some(Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap());

    let id = mailboxes.create(&mut mailbox).expect("create");

    assert!(id.is_assigned());
    let stored = mailboxes.get(id).expect("get").expect("the mailbox");
    assert_eq!(stored, mailbox, "including its synchronization state");
    assert_eq!(stored.role, MailboxRole::Inbox, "resolved from the name");
    assert_eq!(stored.delimiter, Some('/'));
    assert!(stored.selectable && stored.subscribed);
}

#[test]
fn synchronization_state_lives_in_its_own_table() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", None);
    mailbox.generation = Some(Generation::new(7));
    let id = mailboxes.create(&mut mailbox).expect("create");

    let uid_validity: i64 = connection
        .query_row(
            "SELECT uid_validity FROM sync_state WHERE mailbox_id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .expect("the sync_state row exists");
    assert_eq!(uid_validity, 7);

    // The sync engine writes that table directly, in the same transaction as
    // the message writes it describes; the repository must read what it wrote.
    connection
        .execute(
            "UPDATE sync_state SET uid_validity = 8, uid_next = 100 WHERE mailbox_id = ?1",
            [id.get()],
        )
        .expect("update sync state");
    let stored = mailboxes.get(id).expect("get").expect("the mailbox");
    assert_eq!(stored.generation, Some(Generation::new(8)));
    assert_eq!(stored.uid_next, Some(Uid::new(100)));
}

#[test]
fn a_mailbox_that_has_never_been_synced_says_so() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Projects", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");

    let stored = mailboxes.get(id).expect("get").expect("the mailbox");
    assert_eq!(stored.generation, None);
    assert_eq!(stored.uid_next, None);
    assert_eq!(stored.highest_mod_seq, None);
    assert_eq!(stored.last_synced_at, None);
}

#[test]
fn roles_are_stored_with_the_spelling_the_model_documents() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    for (path, expected) in [
        ("INBOX", MailboxRole::Inbox),
        ("Sent Messages", MailboxRole::Sent),
        ("Deleted Messages", MailboxRole::Trash),
        ("Archive", MailboxRole::Archive),
        ("Drafts", MailboxRole::Drafts),
        ("Junk", MailboxRole::Junk),
        ("Projects/Postio", MailboxRole::Regular),
    ] {
        let mut mailbox = Mailbox::new(account_id, path, Some('/'));
        let id = mailboxes.create(&mut mailbox).expect("create");

        let raw: String = connection
            .query_row(
                "SELECT role FROM mailboxes WHERE id = ?1",
                [id.get()],
                |row| row.get(0),
            )
            .expect("read the raw role");
        assert_eq!(raw, expected.as_str(), "{path}");
        assert_eq!(
            mailboxes.get(id).expect("get").expect("the mailbox").role,
            expected,
            "{path}"
        );
    }
}

#[test]
fn a_mailbox_can_be_found_by_path_and_by_role() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).expect("create");
    let mut sent = Mailbox::new(account_id, "Sent Messages", Some('/'));
    mailboxes.create(&mut sent).expect("create");

    assert_eq!(
        mailboxes
            .by_path(account_id, "INBOX")
            .expect("by path")
            .map(|mailbox| mailbox.id),
        Some(inbox_id)
    );
    assert!(
        mailboxes
            .by_path(account_id, "Nowhere")
            .expect("by path")
            .is_none()
    );
    assert_eq!(
        mailboxes
            .by_role(account_id, MailboxRole::Sent)
            .expect("by role")
            .map(|mailbox| mailbox.path),
        Some("Sent Messages".to_owned()),
        "routing is by role, never by name: iCloud calls this Sent Messages"
    );
    assert!(
        mailboxes
            .by_role(account_id, MailboxRole::Junk)
            .expect("by role")
            .is_none()
    );
}

#[test]
fn two_mailboxes_in_one_account_cannot_share_a_path() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut first = Mailbox::new(account_id, "INBOX", Some('/'));
    mailboxes.create(&mut first).expect("create");
    let mut duplicate = Mailbox::new(account_id, "INBOX", Some('/'));

    assert!(
        mailboxes.create(&mut duplicate).is_err(),
        "the same folder must not be mirrored twice"
    );
}

#[test]
fn mailboxes_list_in_hierarchy_order_with_their_children() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).expect("create");
    let mut child = Mailbox::new(account_id, "INBOX/Receipts", Some('/'));
    child.parent_id = Some(inbox_id);
    mailboxes.create(&mut child).expect("create");
    let mut archive = Mailbox::new(account_id, "Archive", Some('/'));
    mailboxes.create(&mut archive).expect("create");

    let all = mailboxes.list_for_account(account_id).expect("list");
    assert_eq!(all.len(), 3);
    assert_eq!(
        all[2].parent_id,
        Some(inbox_id),
        "the child knows its parent"
    );

    let paths: Vec<&str> = all.iter().map(|mailbox| mailbox.path.as_str()).collect();
    assert_eq!(
        paths,
        ["Archive", "INBOX", "INBOX/Receipts"],
        "path order puts a folder immediately before its children"
    );
}

// ---------------------------------------------------------------------------
// Update and delete
// ---------------------------------------------------------------------------

#[test]
fn updating_a_mailbox_changes_its_row_and_its_sync_state() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Projects", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");

    mailbox.name = "Projects (renamed)".to_owned();
    mailbox.subscribed = false;
    mailbox.role = MailboxRole::Archive;
    mailbox.generation = Some(Generation::new(99));
    mailbox.last_synced_at = Some(Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap());
    mailboxes.update(&mailbox).expect("update");

    let stored = mailboxes.get(id).expect("get").expect("the mailbox");
    assert_eq!(stored, mailbox);
}

#[test]
fn a_folder_can_opt_out_of_background_backfill_and_back_in() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Announce", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");
    assert!(
        !mailboxes.backfill_excluded(id).expect("read"),
        "every selectable folder backfills by default (ADR 0016)"
    );

    assert!(
        mailboxes
            .set_backfill_excluded(id, true)
            .expect("set excluded")
    );
    assert!(mailboxes.backfill_excluded(id).expect("read"));
    assert!(
        mailboxes
            .get(id)
            .expect("get")
            .expect("the mailbox")
            .backfill_excluded,
        "the full row agrees with the narrow read"
    );

    assert!(
        mailboxes
            .set_backfill_excluded(id, false)
            .expect("set included")
    );
    assert!(
        !mailboxes.backfill_excluded(id).expect("read"),
        "reversible"
    );
}

#[test]
fn a_mailbox_that_is_not_there_is_not_excluded() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let mailboxes = MailboxRepository::new(&connection);

    assert!(
        !mailboxes
            .backfill_excluded(MailboxId::new(9999))
            .expect("read")
    );
    assert!(
        !mailboxes
            .set_backfill_excluded(MailboxId::new(9999), true)
            .expect("set")
    );
}

#[test]
fn deleting_a_mailbox_takes_its_messages_and_its_sync_state() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");
    insert_message(&connection, id, "\\Seen");

    assert!(mailboxes.delete(id).expect("delete"));

    for table in ["mailboxes", "messages", "sync_state"] {
        let remaining: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(remaining, 0, "{table}");
    }
    assert!(!mailboxes.delete(id).expect("delete again"));
}

// ---------------------------------------------------------------------------
// Acceptance: counts for the sidebar, correct after flag changes
// ---------------------------------------------------------------------------

#[test]
fn recounting_fills_in_the_sidebars_numbers() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");
    insert_message(&connection, id, "\\Seen");
    insert_message(&connection, id, "");
    insert_message(&connection, id, "\\Flagged");
    insert_message(&connection, id, "\\Seen \\Flagged");

    let counts = mailboxes.recount(id).expect("recount");

    assert_eq!(
        counts,
        MailboxCounts {
            total: 4,
            unread: 2,
            flagged: 2,
            snoozed: 0
        }
    );
    assert_eq!(
        mailboxes.get(id).expect("get").expect("the mailbox").counts,
        counts,
        "the numbers are cached on the row, so the sidebar never counts rows"
    );
}

#[test]
fn counts_stay_correct_after_a_flag_change() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");
    insert_message(&connection, id, "");
    insert_message(&connection, id, "");
    assert_eq!(mailboxes.recount(id).expect("recount").unread, 2);

    connection
        .execute(
            "UPDATE messages SET seen = 1, flags = '\\Seen' WHERE id = (SELECT min(id) FROM messages)",
            [],
        )
        .expect("mark one as read");

    let counts = mailboxes.recount(id).expect("recount");
    assert_eq!(counts.unread, 1);
    assert_eq!(counts.total, 2, "reading a message does not remove it");
}

#[test]
fn a_message_deleted_locally_is_not_in_the_counts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");
    insert_message(&connection, id, "");
    insert_message(&connection, id, "");
    connection
        .execute(
            "UPDATE messages SET deleted_locally = 1 WHERE id = (SELECT min(id) FROM messages)",
            [],
        )
        .expect("hide one pending a remote delete");

    let counts = mailboxes.recount(id).expect("recount");

    assert_eq!(
        counts,
        MailboxCounts {
            total: 1,
            unread: 1,
            flagged: 0,
            snoozed: 0
        },
        "the list hides it, so the sidebar must not count it"
    );
}

#[test]
fn every_mailbox_in_an_account_can_be_recounted_at_once() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).expect("create");
    let mut archive = Mailbox::new(account_id, "Archive", Some('/'));
    let archive_id = mailboxes.create(&mut archive).expect("create");
    insert_message(&connection, inbox_id, "");
    insert_message(&connection, archive_id, "\\Seen");
    insert_message(&connection, archive_id, "\\Seen");

    mailboxes.recount_account(account_id).expect("recount all");

    let listed = mailboxes.list_for_account(account_id).expect("list");
    let archive = listed.iter().find(|m| m.id == archive_id).expect("archive");
    let inbox = listed.iter().find(|m| m.id == inbox_id).expect("inbox");
    assert_eq!(inbox.counts.unread, 1);
    assert_eq!(archive.counts.total, 2);
    assert_eq!(archive.counts.unread, 0);

    assert_eq!(
        mailboxes
            .account_counts(account_id)
            .expect("account counts"),
        MailboxCounts {
            total: 3,
            unread: 1,
            flagged: 0,
            snoozed: 0
        },
        "the account row in the sidebar sums its folders"
    );
}

#[test]
fn counts_can_be_written_directly_for_a_server_reported_status() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account_id = seeded_account(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).expect("create");

    // A STATUS response tells us about messages we have not fetched yet, so
    // the counts have to be settable without any local rows behind them.
    let reported = MailboxCounts {
        total: 12_000,
        unread: 37,
        flagged: 4,
        snoozed: 0,
    };
    mailboxes.set_counts(id, reported).expect("set counts");

    assert_eq!(
        mailboxes.get(id).expect("get").expect("the mailbox").counts,
        reported
    );
}

#[test]
fn reading_a_mailbox_that_is_not_there_is_none() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let mailboxes = MailboxRepository::new(&connection);

    assert!(mailboxes.get(MailboxId::new(404)).expect("get").is_none());
    assert!(
        mailboxes
            .list_for_account(AccountId::new(404))
            .expect("list")
            .is_empty()
    );
}
