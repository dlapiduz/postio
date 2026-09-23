//! Mailboxes: CRUD, special-use roles, sync state, and the sidebar's counts.

use chrono::{TimeZone, Utc};
use postio_storage::Connection;
use postio_storage::sql::bind;

use postio_model::{
    Account, AccountId, EmailAddress, Generation, Mailbox, MailboxCounts, MailboxId, MailboxRole,
    ModSeq, Uid,
};
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_storage::test_support;

async fn seeded_account(connection: &Connection) -> AccountId {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "test@example.com"),
    );
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create an account")
}

/// Inserts a message straight into the table. The message repository is a
/// separate bead; the counts have to be right before it exists.
async fn insert_message(connection: &Connection, mailbox: MailboxId, flags: &str) {
    let seen = i64::from(flags.contains("\\Seen"));
    let flagged = i64::from(flags.contains("\\Flagged"));
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, flags, seen, flagged)
             SELECT account_id, id, 0, ?2, ?3, ?4 FROM mailboxes WHERE id = ?1",
            bind![mailbox.get(), flags, seen, flagged],
        )
        .await
        .expect("insert a message");
}

// ---------------------------------------------------------------------------
// Create and read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_mailbox_round_trips_through_the_database() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    mailbox.generation = Some(Generation::new(1_707_000_000));
    mailbox.uid_next = Some(Uid::new(4_412));
    mailbox.highest_mod_seq = Some(ModSeq::new(90_210));
    mailbox.last_synced_at = Some(Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap());

    let id = mailboxes.create(&mut mailbox).await.expect("create");

    assert!(id.is_assigned());
    let stored = mailboxes.get(id).await.expect("get").expect("the mailbox");
    assert_eq!(stored, mailbox, "including its synchronization state");
    assert_eq!(stored.role, MailboxRole::Inbox, "resolved from the name");
    assert_eq!(stored.delimiter, Some('/'));
    assert!(stored.selectable && stored.subscribed);
}

#[tokio::test]
async fn synchronization_state_lives_in_its_own_table() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", None);
    mailbox.generation = Some(Generation::new(7));
    let id = mailboxes.create(&mut mailbox).await.expect("create");

    let uid_validity: i64 = postio_storage::sql::one(
        &connection,
        "SELECT uid_validity FROM sync_state WHERE mailbox_id = ?1",
        [id.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the sync_state row exists");
    assert_eq!(uid_validity, 7);

    // The sync engine writes that table directly, in the same transaction as
    // the message writes it describes; the repository must read what it wrote.
    connection
        .execute(
            "UPDATE sync_state SET uid_validity = 8, uid_next = 100 WHERE mailbox_id = ?1",
            [id.get()],
        )
        .await
        .expect("update sync state");
    let stored = mailboxes.get(id).await.expect("get").expect("the mailbox");
    assert_eq!(stored.generation, Some(Generation::new(8)));
    assert_eq!(stored.uid_next, Some(Uid::new(100)));
}

#[tokio::test]
async fn a_mailbox_that_has_never_been_synced_says_so() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Projects", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");

    let stored = mailboxes.get(id).await.expect("get").expect("the mailbox");
    assert_eq!(stored.generation, None);
    assert_eq!(stored.uid_next, None);
    assert_eq!(stored.highest_mod_seq, None);
    assert_eq!(stored.last_synced_at, None);
}

#[tokio::test]
async fn roles_are_stored_with_the_spelling_the_model_documents() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
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
        let id = mailboxes.create(&mut mailbox).await.expect("create");

        let raw: String = postio_storage::sql::one(
            &connection,
            "SELECT role FROM mailboxes WHERE id = ?1",
            [id.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("read the raw role");
        assert_eq!(raw, expected.as_str(), "{path}");
        assert_eq!(
            mailboxes
                .get(id)
                .await
                .expect("get")
                .expect("the mailbox")
                .role,
            expected,
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_mailbox_can_be_found_by_path_and_by_role() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).await.expect("create");
    let mut sent = Mailbox::new(account_id, "Sent Messages", Some('/'));
    mailboxes.create(&mut sent).await.expect("create");

    assert_eq!(
        mailboxes
            .by_path(account_id, "INBOX")
            .await
            .expect("by path")
            .map(|mailbox| mailbox.id),
        Some(inbox_id)
    );
    assert!(
        mailboxes
            .by_path(account_id, "Nowhere")
            .await
            .expect("by path")
            .is_none()
    );
    assert_eq!(
        mailboxes
            .by_role(account_id, MailboxRole::Sent)
            .await
            .expect("by role")
            .map(|mailbox| mailbox.path),
        Some("Sent Messages".to_owned()),
        "routing is by role, never by name: iCloud calls this Sent Messages"
    );
    assert!(
        mailboxes
            .by_role(account_id, MailboxRole::Junk)
            .await
            .expect("by role")
            .is_none()
    );
}

#[tokio::test]
async fn two_mailboxes_in_one_account_cannot_share_a_path() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut first = Mailbox::new(account_id, "INBOX", Some('/'));
    mailboxes.create(&mut first).await.expect("create");
    let mut duplicate = Mailbox::new(account_id, "INBOX", Some('/'));

    assert!(
        mailboxes.create(&mut duplicate).await.is_err(),
        "the same folder must not be mirrored twice"
    );
}

#[tokio::test]
async fn mailboxes_list_in_hierarchy_order_with_their_children() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).await.expect("create");
    let mut child = Mailbox::new(account_id, "INBOX/Receipts", Some('/'));
    child.parent_id = Some(inbox_id);
    mailboxes.create(&mut child).await.expect("create");
    let mut archive = Mailbox::new(account_id, "Archive", Some('/'));
    mailboxes.create(&mut archive).await.expect("create");

    let all = mailboxes.list_for_account(account_id).await.expect("list");
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

#[tokio::test]
async fn updating_a_mailbox_changes_its_row_and_its_sync_state() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Projects", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");

    mailbox.name = "Projects (renamed)".to_owned();
    mailbox.subscribed = false;
    mailbox.role = MailboxRole::Archive;
    mailbox.generation = Some(Generation::new(99));
    mailbox.last_synced_at = Some(Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap());
    mailboxes.update(&mailbox).await.expect("update");

    let stored = mailboxes.get(id).await.expect("get").expect("the mailbox");
    assert_eq!(stored, mailbox);
}

#[tokio::test]
async fn a_folder_can_opt_out_of_background_backfill_and_back_in() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "Announce", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");
    assert!(
        !mailboxes.backfill_excluded(id).await.expect("read"),
        "every selectable folder backfills by default (ADR 0016)"
    );

    assert!(
        mailboxes
            .set_backfill_excluded(id, true)
            .await
            .expect("set excluded")
    );
    assert!(mailboxes.backfill_excluded(id).await.expect("read"));
    assert!(
        mailboxes
            .get(id)
            .await
            .expect("get")
            .expect("the mailbox")
            .backfill_excluded,
        "the full row agrees with the narrow read"
    );

    assert!(
        mailboxes
            .set_backfill_excluded(id, false)
            .await
            .expect("set included")
    );
    assert!(
        !mailboxes.backfill_excluded(id).await.expect("read"),
        "reversible"
    );
}

#[tokio::test]
async fn a_mailbox_that_is_not_there_is_not_excluded() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let mailboxes = MailboxRepository::new(&connection);

    assert!(
        !mailboxes
            .backfill_excluded(MailboxId::new(9999))
            .await
            .expect("read")
    );
    assert!(
        !mailboxes
            .set_backfill_excluded(MailboxId::new(9999), true)
            .await
            .expect("set")
    );
}

#[tokio::test]
async fn deleting_a_mailbox_takes_its_messages_and_its_sync_state() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");
    insert_message(&connection, id, "\\Seen").await;

    assert!(mailboxes.delete(id).await.expect("delete"));

    for table in ["mailboxes", "messages", "sync_state"] {
        let remaining: i64 = postio_storage::sql::one(
            &connection,
            &format!("SELECT count(*) FROM {table}"),
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("count");
        assert_eq!(remaining, 0, "{table}");
    }
    assert!(!mailboxes.delete(id).await.expect("delete again"));
}

// ---------------------------------------------------------------------------
// Acceptance: counts for the sidebar, correct after flag changes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recounting_fills_in_the_sidebars_numbers() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");
    insert_message(&connection, id, "\\Seen").await;
    insert_message(&connection, id, "").await;
    insert_message(&connection, id, "\\Flagged").await;
    insert_message(&connection, id, "\\Seen \\Flagged").await;

    let counts = mailboxes.recount(id).await.expect("recount");

    assert_eq!(
        counts,
        MailboxCounts {
            total: 4,
            unread: 2,
            flagged: 2,
            snoozed: 0,
            attention: 0,
        }
    );
    assert_eq!(
        mailboxes
            .get(id)
            .await
            .expect("get")
            .expect("the mailbox")
            .counts,
        counts,
        "the numbers are cached on the row, so the sidebar never counts rows"
    );
}

#[tokio::test]
async fn counts_stay_correct_after_a_flag_change() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");
    insert_message(&connection, id, "").await;
    insert_message(&connection, id, "").await;
    assert_eq!(mailboxes.recount(id).await.expect("recount").unread, 2);

    connection
        .execute(
            "UPDATE messages SET seen = 1, flags = '\\Seen' WHERE id = (SELECT min(id) FROM messages)",
            (),
        )
        .await
        .expect("mark one as read");

    let counts = mailboxes.recount(id).await.expect("recount");
    assert_eq!(counts.unread, 1);
    assert_eq!(counts.total, 2, "reading a message does not remove it");
}

#[tokio::test]
async fn a_message_deleted_locally_is_not_in_the_counts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");
    insert_message(&connection, id, "").await;
    insert_message(&connection, id, "").await;
    connection
        .execute(
            "UPDATE messages SET deleted_locally = 1 WHERE id = (SELECT min(id) FROM messages)",
            (),
        )
        .await
        .expect("hide one pending a remote delete");

    let counts = mailboxes.recount(id).await.expect("recount");

    assert_eq!(
        counts,
        MailboxCounts {
            total: 1,
            unread: 1,
            flagged: 0,
            snoozed: 0,
            attention: 0,
        },
        "the list hides it, so the sidebar must not count it"
    );
}

#[tokio::test]
async fn every_mailbox_in_an_account_can_be_recounted_at_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut inbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let inbox_id = mailboxes.create(&mut inbox).await.expect("create");
    let mut archive = Mailbox::new(account_id, "Archive", Some('/'));
    let archive_id = mailboxes.create(&mut archive).await.expect("create");
    insert_message(&connection, inbox_id, "").await;
    insert_message(&connection, archive_id, "\\Seen").await;
    insert_message(&connection, archive_id, "\\Seen").await;

    mailboxes
        .recount_account(account_id)
        .await
        .expect("recount all");

    let listed = mailboxes.list_for_account(account_id).await.expect("list");
    let archive = listed.iter().find(|m| m.id == archive_id).expect("archive");
    let inbox = listed.iter().find(|m| m.id == inbox_id).expect("inbox");
    assert_eq!(inbox.counts.unread, 1);
    assert_eq!(archive.counts.total, 2);
    assert_eq!(archive.counts.unread, 0);

    assert_eq!(
        mailboxes
            .account_counts(account_id)
            .await
            .expect("account counts"),
        MailboxCounts {
            total: 3,
            unread: 1,
            flagged: 0,
            snoozed: 0,
            attention: 0,
        },
        "the account row in the sidebar sums its folders"
    );
}

#[tokio::test]
async fn counts_can_be_written_directly_for_a_server_reported_status() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account_id, "INBOX", Some('/'));
    let id = mailboxes.create(&mut mailbox).await.expect("create");

    // A STATUS response tells us about messages we have not fetched yet, so
    // the counts have to be settable without any local rows behind them.
    let reported = MailboxCounts {
        total: 12_000,
        unread: 37,
        flagged: 4,
        snoozed: 0,
        attention: 0,
    };
    mailboxes
        .set_counts(id, reported)
        .await
        .expect("set counts");

    assert_eq!(
        mailboxes
            .get(id)
            .await
            .expect("get")
            .expect("the mailbox")
            .counts,
        reported
    );
}

#[tokio::test]
async fn reading_a_mailbox_that_is_not_there_is_none() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let mailboxes = MailboxRepository::new(&connection);

    assert!(
        mailboxes
            .get(MailboxId::new(404))
            .await
            .expect("get")
            .is_none()
    );
    assert!(
        mailboxes
            .list_for_account(AccountId::new(404))
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn by_role_never_answers_with_a_folder_the_server_no_longer_has() {
    // #943. Discovery retires a folder the server stopped listing by clearing
    // `selectable` and nothing else, so a retired row keeps its role. The
    // role lookup is what the send path files a copy through; answering with
    // a retired row sends the copy to a folder that does not exist.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account_id = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    // Sorts first, so it is what `ORDER BY path LIMIT 1` would pick.
    let mut retired = Mailbox::new(account_id, "Sent", Some('/'));
    retired.role = MailboxRole::Sent;
    retired.selectable = false;
    mailboxes.create(&mut retired).await.expect("create");
    let mut live = Mailbox::new(account_id, "Sent Items", Some('/'));
    live.role = MailboxRole::Sent;
    mailboxes.create(&mut live).await.expect("create");

    assert_eq!(
        mailboxes
            .by_role(account_id, MailboxRole::Sent)
            .await
            .expect("by role")
            .map(|mailbox| mailbox.path),
        Some("Sent Items".to_owned()),
        "a role points at a folder that can be opened, or at nothing"
    );
}

// ── A view is not storable (spec 003, FR-041) ───────────────────────────────

/// The three roles that are views over messages filed elsewhere, not folders.
///
/// Before spec 003 this was enforced by accident: `Snoozed` could not be
/// stored only because `0001_initial_schema.sql`'s `CHECK` happened not to
/// list it, and nothing said why. A `CHECK` violation is also the wrong error
/// for the caller -- it says "constraint failed", not "that role names no
/// folder" -- so the repository refuses first and names the role.
#[tokio::test]
async fn a_view_role_cannot_be_stored_as_a_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    // Not `Flagged`: RFC 6154 defines `\Flagged`, so a server can really have
    // that folder and `flagged` is in the schema's CHECK for that reason.
    // `Snoozed` and `Outbox` have no attribute and no server can advertise one.
    for role in [MailboxRole::Snoozed, MailboxRole::Outbox] {
        let mut mailbox = Mailbox::new(account, "Somewhere", None);
        mailbox.role = role;
        let outcome = mailboxes.create(&mut mailbox);

        let message = match outcome.await {
            Err(error) => error.to_string(),
            Ok(_) => panic!("{role:?} is a view over messages filed elsewhere; it stored anyway"),
        };
        assert!(
            message.contains(role.as_str()),
            "the error has to name the role, or the caller cannot tell which of \
             its mailboxes was wrong: {message}"
        );
    }
}

#[tokio::test]
async fn a_view_role_cannot_be_stored_by_updating_a_folder_into_one() {
    // The other way in. `create` refusing is worth nothing if `update` lets a
    // real folder be re-roled into a view afterwards.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let mut mailbox = Mailbox::new(account, "Projects", None);
    mailboxes
        .create(&mut mailbox)
        .await
        .expect("an ordinary folder stores");

    mailbox.role = MailboxRole::Outbox;
    let message = match mailboxes.update(&mailbox).await {
        Err(error) => error.to_string(),
        Ok(()) => panic!("a stored folder was re-roled into a view"),
    };
    // Tightened deliberately: SQLite's own `CHECK` already refuses this, so
    // asserting only `is_err()` would pass without the repository doing
    // anything and would never have been seen red. What is being tested is
    // that the *caller* is told which role was wrong, which a constraint
    // violation does not say.
    assert!(
        message.contains("outbox"),
        "the error has to name the role rather than report a constraint: {message}"
    );
}

#[tokio::test]
async fn every_reserved_role_still_stores() {
    // The other half of the rule: refusing views must not refuse folders.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = seeded_account(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    for role in MailboxRole::RESERVED {
        let mut mailbox = Mailbox::new(account, role.as_str(), None);
        mailbox.role = role;
        mailboxes
            .create(&mut mailbox)
            .await
            .unwrap_or_else(|error| panic!("{role:?} is a folder and must store: {error}"));
    }
}

// -- when a folder last synced (#1281) ---------------------------------------

#[tokio::test]
async fn recording_a_sync_sets_the_time_and_leaves_the_counts_alone() {
    // The counts are maintained by the schema's `messages_count_*` triggers.
    // Writing the whole row back from a `Mailbox` read minutes ago would
    // overwrite them with whatever was true then, which is why this is a
    // narrow UPDATE.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let repository = MailboxRepository::new(&connection);

    repository
        .set_counts(
            inbox,
            MailboxCounts {
                total: 4_985,
                unread: 37,
                flagged: 2,
                snoozed: 0,
                // Not a message count: the sidebar's feed fills it in before
                // a row is drawn, and a STATUS-shaped write has nothing to
                // say about it.
                attention: 0,
            },
        )
        .await
        .expect("counts");

    let at = Utc.with_ymd_and_hms(2026, 9, 6, 18, 51, 0).unwrap();
    repository
        .record_sync(inbox, at)
        .await
        .expect("the time is recorded");

    let stored = repository
        .get(inbox)
        .await
        .expect("a read")
        .expect("the mailbox");
    assert_eq!(stored.last_synced_at, Some(at));
    assert_eq!(stored.counts.total, 4_985, "the counts are untouched");
    assert_eq!(stored.counts.unread, 37);
    let _ = account;
}

#[tokio::test]
async fn a_second_pass_moves_the_time_forward() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;
    let repository = MailboxRepository::new(&connection);

    let first = Utc.with_ymd_and_hms(2026, 9, 6, 9, 0, 0).unwrap();
    let second = Utc.with_ymd_and_hms(2026, 9, 6, 18, 0, 0).unwrap();
    repository.record_sync(inbox, first).await.expect("first");
    repository.record_sync(inbox, second).await.expect("second");

    assert_eq!(
        repository
            .get(inbox)
            .await
            .expect("a read")
            .expect("it")
            .last_synced_at,
        Some(second)
    );
}

#[tokio::test]
async fn recording_a_sync_for_a_folder_that_is_gone_says_so() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let repository = MailboxRepository::new(&connection);

    let error = repository
        .record_sync(MailboxId::new(404), Utc::now())
        .await
        .expect_err("there is no such folder");
    assert!(matches!(error, postio_storage::Error::NotFound { .. }));
}
