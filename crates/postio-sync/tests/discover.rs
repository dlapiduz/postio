//! Turning the server's folder list into local rows.
//!
//! This is the step whose absence made the first live run report success over
//! an empty mailbox: every layer read the local `mailboxes` table, and nothing
//! ever wrote it. See `postio-755`.

use postio_imap::backend::{MailBackend, MockBackend, MockMailbox};
use postio_model::{Account, EmailAddress, MailboxRole, Message};
use postio_storage::repository::{AccountRepository, MailboxRepository, MessageRepository};
use postio_storage::test_support;
use postio_sync::discover::discover;
use rusqlite::Connection;

fn an_account(connection: &Connection) -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("create account");
    account
}

/// A server with the folders a real account has, roles carried by the RFC 6154
/// attributes rather than by their names.
async fn a_server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Archive").attributes(["\\Archive"]))
        .mailbox(MockMailbox::new("Sent Messages").attributes(["\\Sent"]))
        .mailbox(MockMailbox::new("Deleted Messages").attributes(["\\Trash"]))
        .build();
    backend.connect().await.expect("connect");
    backend
}

fn paths(connection: &Connection, account: &Account) -> Vec<String> {
    let mut paths: Vec<String> = MailboxRepository::new(connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .map(|mailbox| mailbox.path)
        .collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn discovery_writes_the_servers_folders_into_the_local_table() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    let report = discover(&connection, &backend, account.id)
        .await
        .expect("discover");

    assert_eq!(report.added, 4, "{report:?}");
    assert_eq!(
        paths(&connection, &account),
        vec!["Archive", "Deleted Messages", "INBOX", "Sent Messages"]
    );
}

#[tokio::test]
async fn a_folders_role_comes_from_the_servers_attributes_not_its_name() {
    // "Sent Messages" and "Deleted Messages" are what some servers call them.
    // A client that matched on the English word would file mail into the wrong
    // folder on every account that does not speak English.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id)
        .await
        .expect("discover");

    let mailboxes = MailboxRepository::new(&connection);
    for (role, path) in [
        (MailboxRole::Inbox, "INBOX"),
        (MailboxRole::Archive, "Archive"),
        (MailboxRole::Sent, "Sent Messages"),
        (MailboxRole::Trash, "Deleted Messages"),
    ] {
        let found = mailboxes
            .by_role(account.id, role)
            .expect("by role")
            .unwrap_or_else(|| panic!("no folder resolved to {role:?}"));
        assert_eq!(found.path, path);
    }
}

#[tokio::test]
async fn discovering_twice_keeps_the_same_rows() {
    // Everything else points at these ids: sync state, messages, the queue.
    // A discovery that reinserted would orphan every one of them.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id)
        .await
        .expect("first pass");
    let before: Vec<_> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .map(|mailbox| (mailbox.path, mailbox.id))
        .collect();

    let report = discover(&connection, &backend, account.id)
        .await
        .expect("second pass");

    let after: Vec<_> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .map(|mailbox| (mailbox.path, mailbox.id))
        .collect();

    assert_eq!(report.added, 0, "{report:?}");
    assert_eq!(
        before, after,
        "the ids everything else points at must not move"
    );
}

#[tokio::test]
async fn discovery_preserves_what_a_sync_pass_recorded() {
    // The rows carry sync state — UIDVALIDITY, the highest MODSEQ — and losing
    // it would turn every reconnection into a full re-enumeration of every
    // folder.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id)
        .await
        .expect("first pass");

    let mailboxes = MailboxRepository::new(&connection);
    let mut inbox = mailboxes
        .by_role(account.id, MailboxRole::Inbox)
        .expect("by role")
        .expect("an inbox");
    inbox.uid_validity = Some(postio_model::UidValidity::new(42));
    inbox.highest_mod_seq = Some(postio_model::ModSeq::new(900));
    mailboxes.update(&inbox).expect("record a synced inbox");

    discover(&connection, &backend, account.id)
        .await
        .expect("second pass");

    let after = mailboxes.get(inbox.id).expect("get").expect("still there");
    assert_eq!(after.uid_validity, Some(postio_model::UidValidity::new(42)));
    assert_eq!(after.highest_mod_seq, Some(postio_model::ModSeq::new(900)));
}

#[tokio::test]
async fn a_folder_the_server_no_longer_lists_keeps_its_mail() {
    // The row is not deleted: `messages.mailbox_id` cascades, so deleting a
    // folder because one LIST did not mention it would delete the user's mail.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id)
        .await
        .expect("first pass");
    let mailboxes = MailboxRepository::new(&connection);
    let archive = mailboxes
        .by_role(account.id, MailboxRole::Archive)
        .expect("by role")
        .expect("an archive");

    let mut message = Message::new(account.id, archive.id, chrono::Utc::now());
    message.subject = Some("Filed away years ago".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("file a message in it");

    // The folder is renamed or removed on the server.
    let smaller = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent Messages").attributes(["\\Sent"]))
        .build();
    smaller.connect().await.expect("connect");

    let report = discover(&connection, &smaller, account.id)
        .await
        .expect("discover");

    assert_eq!(report.vanished, 2, "{report:?}");
    let after = mailboxes
        .get(archive.id)
        .expect("get")
        .expect("the row is still there");
    assert!(
        !after.selectable,
        "a folder that is not on the server cannot be opened, and the engine \
         reads exactly that to decide what to sync and watch"
    );
    assert!(
        MessageRepository::new(&connection)
            .get(message.id)
            .expect("get")
            .is_some(),
        "and the mail in it survives"
    );
}

#[tokio::test]
async fn a_folder_that_comes_back_is_usable_again() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);

    let smaller = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    smaller.connect().await.expect("connect");
    discover(&connection, &smaller, account.id)
        .await
        .expect("first pass");

    let full = a_server().await;
    discover(&connection, &full, account.id)
        .await
        .expect("second pass");

    let inbox = MailboxRepository::new(&connection)
        .by_role(account.id, MailboxRole::Inbox)
        .expect("by role")
        .expect("an inbox");
    assert!(inbox.selectable);
}

#[tokio::test]
async fn an_empty_listing_is_not_read_as_every_folder_being_gone() {
    // A server that answers LIST with nothing, or a listing that failed part
    // way, must not empty the sidebar. Nothing is evidence of nothing.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id)
        .await
        .expect("first pass");

    let empty = MockBackend::builder().build();
    empty.connect().await.expect("connect");
    let report = discover(&connection, &empty, account.id)
        .await
        .expect("discover");

    assert_eq!(report.vanished, 0, "{report:?}");
    assert_eq!(paths(&connection, &account).len(), 4);
}

#[tokio::test]
async fn a_child_folder_is_linked_to_its_parent() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").delimiter('/'))
        .mailbox(MockMailbox::new("INBOX/Receipts").delimiter('/'))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id)
        .await
        .expect("discover");

    let mailboxes = MailboxRepository::new(&connection);
    let inbox = mailboxes
        .by_path(account.id, "INBOX")
        .expect("by path")
        .expect("an inbox");
    let child = mailboxes
        .by_path(account.id, "INBOX/Receipts")
        .expect("by path")
        .expect("the child");

    assert_eq!(child.parent_id, Some(inbox.id));
    assert_eq!(child.name, "Receipts", "the leaf, not the whole path");
}

#[tokio::test]
async fn a_folder_that_cannot_hold_messages_is_recorded_as_such() {
    // `\Noselect` is how a server spells "this is only a level in the
    // hierarchy". Selecting one is an error, so the engine must not try.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Lists").attributes(["\\Noselect"]))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id)
        .await
        .expect("discover");

    let lists = MailboxRepository::new(&connection)
        .by_path(account.id, "Lists")
        .expect("by path")
        .expect("the folder");
    assert!(!lists.selectable);
}
