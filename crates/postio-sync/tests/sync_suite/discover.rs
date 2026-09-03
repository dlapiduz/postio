//! Turning the server's folder list into local rows.
//!
//! This is the step whose absence made the first live run report success over
//! an empty mailbox: every layer read the local `mailboxes` table, and nothing
//! ever wrote it. See `postio-755`.

use postio_account::backend::{MailBackend, MockBackend, MockMailbox};
use postio_model::{Account, EmailAddress, MailboxRole, Message, RoleOverrides};
use postio_storage::repository::{
    AccountRepository, MailboxRepository, MailboxRoleRepository, MessageRepository,
};
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

    let report = discover(&connection, &backend, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");
    let before: Vec<_> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .map(|mailbox| (mailbox.path, mailbox.id))
        .collect();

    let report = discover(&connection, &backend, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let mailboxes = MailboxRepository::new(&connection);
    let mut inbox = mailboxes
        .by_role(account.id, MailboxRole::Inbox)
        .expect("by role")
        .expect("an inbox");
    inbox.generation = Some(postio_model::Generation::new(42));
    inbox.highest_mod_seq = Some(postio_model::ModSeq::new(900));
    mailboxes.update(&inbox).expect("record a synced inbox");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    let after = mailboxes.get(inbox.id).expect("get").expect("still there");
    assert_eq!(after.generation, Some(postio_model::Generation::new(42)));
    assert_eq!(after.highest_mod_seq, Some(postio_model::ModSeq::new(900)));
}

#[tokio::test]
async fn discovery_does_not_reset_a_folders_backfill_exclusion() {
    // ADR 0016, #350: excluding a folder from background backfill is a local
    // preference, the same shape `signature_id` (#394) already is. A LIST
    // response says nothing about it, so a reconnection must not silently
    // re-include a folder the user deliberately excluded.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let mailboxes = MailboxRepository::new(&connection);
    let inbox = mailboxes
        .by_role(account.id, MailboxRole::Inbox)
        .expect("by role")
        .expect("an inbox");
    mailboxes
        .set_backfill_excluded(inbox.id, true)
        .expect("exclude the inbox");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    assert!(
        mailboxes.backfill_excluded(inbox.id).expect("read"),
        "a resync must not reach a decision the server was never asked about"
    );
}

#[tokio::test]
async fn a_folder_the_server_no_longer_lists_keeps_its_mail() {
    // The row is not deleted: `messages.mailbox_id` cascades, so deleting a
    // folder because one LIST did not mention it would delete the user's mail.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
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

    let report = discover(&connection, &smaller, account.id, &RoleOverrides::default())
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
    discover(&connection, &smaller, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let full = a_server().await;
    discover(&connection, &full, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let empty = MockBackend::builder().build();
    empty.connect().await.expect("connect");
    let report = discover(&connection, &empty, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
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

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    let lists = MailboxRepository::new(&connection)
        .by_path(account.id, "Lists")
        .expect("by path")
        .expect("the folder");
    assert!(!lists.selectable);
}

// ── Explicit role overrides (#164) ───────────────────────────────────────

/// A server that gives no help at all: no `SPECIAL-USE`, and folder names in
/// a language `match_name` has never been taught. Before an override existed,
/// `a` and `d` refused on this account permanently.
async fn an_unhelpful_server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Vecchia Posta"))
        .mailbox(MockMailbox::new("Cestino"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

fn role_of(connection: &Connection, account: &Account, path: &str) -> Option<MailboxRole> {
    MailboxRepository::new(connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .find(|mailbox| mailbox.path == path)
        .map(|mailbox| mailbox.role)
}

#[tokio::test]
async fn an_override_gives_a_role_to_a_folder_nothing_else_could_name() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = an_unhelpful_server().await;

    // Without one, exactly the state the issue describes: no archive folder,
    // so every role-driven verb refuses.
    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");
    assert_eq!(
        role_of(&connection, &account, "Vecchia Posta"),
        Some(MailboxRole::Regular),
        "nothing about this folder is guessable, which is the premise"
    );

    let overrides = RoleOverrides::from_pairs([
        (MailboxRole::Archive, "Vecchia Posta"),
        (MailboxRole::Trash, "Cestino"),
    ]);
    discover(&connection, &backend, account.id, &overrides)
        .await
        .expect("discover");

    assert_eq!(
        role_of(&connection, &account, "Vecchia Posta"),
        Some(MailboxRole::Archive)
    );
    assert_eq!(
        role_of(&connection, &account, "Cestino"),
        Some(MailboxRole::Trash)
    );
    assert_eq!(
        role_of(&connection, &account, "INBOX"),
        Some(MailboxRole::Inbox),
        "the inbox is still the inbox; an override elsewhere does not disturb it"
    );
}

#[tokio::test]
async fn an_override_outranks_what_the_server_said() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_server().await;

    // This server advertises `\Archive` on the folder called Archive. The
    // user has looked and disagreed, and there is no reading of that in which
    // Postio knows better.
    let overrides = RoleOverrides::from_pairs([(MailboxRole::Junk, "Archive")]);
    discover(&connection, &backend, account.id, &overrides)
        .await
        .expect("discover");

    assert_eq!(
        role_of(&connection, &account, "Archive"),
        Some(MailboxRole::Junk)
    );
}

/// **The behaviour the issue flagged as undecided, decided.**
///
/// Changing a mapping re-labels folders. It never moves mail.
///
/// A role is a property of a *mailbox* row — which folder plays which part —
/// and not of any message. Messages live in folders, and re-pointing `archive`
/// at a different folder says nothing about where anything already is. The
/// alternative, moving mail to match, would mean Postio issuing IMAP moves
/// that the user never asked for, on a config edit, against the rule that
/// nothing leaves this machine unasked. It would also be irreversible: change
/// the line back and the labels swap back, but moved mail stays moved.
#[tokio::test]
async fn remapping_a_role_moves_the_label_and_never_the_mail() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Archive").attributes(["\\Archive"]))
        .mailbox(MockMailbox::new("Vecchia Posta"))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    // Put a message in the folder that is the archive today.
    let archive = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .find(|mailbox| mailbox.path == "Archive")
        .expect("an Archive row");
    let mut message = Message::new(account.id, archive.id, chrono::Utc::now());
    message.subject = Some("filed under the old archive".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create message");

    // Now point `archive` somewhere else.
    let overrides = RoleOverrides::from_pairs([(MailboxRole::Archive, "Vecchia Posta")]);
    discover(&connection, &backend, account.id, &overrides)
        .await
        .expect("discover");

    assert_eq!(
        role_of(&connection, &account, "Vecchia Posta"),
        Some(MailboxRole::Archive),
        "the new folder wears the role"
    );
    assert_eq!(
        role_of(&connection, &account, "Archive"),
        Some(MailboxRole::Regular),
        "and the old one gives it up — two folders cannot both be the archive, \
         because `by_role` returns one"
    );

    let still_there = MessageRepository::new(&connection)
        .get(message.id)
        .expect("read");
    assert_eq!(
        still_there.map(|m| m.mailbox_id),
        Some(archive.id),
        "the mail did not move. Re-pointing a role is a relabelling; moving \
         mail would be a network operation nobody asked for, and unlike a \
         relabelling it could not be undone by editing the line back."
    );
}

#[tokio::test]
async fn a_role_follows_the_folder_when_the_server_renames_it() {
    // #943. The Sent folder is renamed on the server (by another client, or
    // by the provider). The old row is retired and keeps its role; the new
    // row is born with the same role; and `by_role` picks between them by
    // path order. "Sent" sorts before "Sent Items", so every sent copy from
    // here on is filed into a folder the server no longer has.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);

    let before = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent").attributes(["\\Sent"]))
        .build();
    before.connect().await.expect("connect");
    discover(&connection, &before, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let after = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent Items").attributes(["\\Sent"]))
        .build();
    after.connect().await.expect("connect");
    discover(&connection, &after, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    let sent = MailboxRepository::new(&connection)
        .by_role(account.id, MailboxRole::Sent)
        .expect("by role")
        .expect("the account still has a Sent folder");
    assert_eq!(
        sent.path, "Sent Items",
        "the role belongs to the folder the server has, not the one it had"
    );
    assert!(
        sent.selectable,
        "a folder the server does not list cannot be the place mail is filed"
    );
}

#[tokio::test]
async fn one_folder_per_role_survives_discovery() {
    // #943, as found on a live account: the server has its own Sent folder
    // and a user folder another client created that merely looks like one.
    // The IMAP edge arbitrates the pair (`resolve_roles`) and demotes the
    // loser; discovery then throws that verdict away by re-deriving the
    // role from the name, and both rows wear it. `by_role` picks between
    // them by path order, and "Sent" sorts before "Sent Messages".
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent Messages").attributes(["\\Sent"]))
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    let mailboxes = MailboxRepository::new(&connection);
    let sent: Vec<String> = mailboxes
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .filter(|mailbox| mailbox.role == MailboxRole::Sent)
        .map(|mailbox| mailbox.path)
        .collect();
    assert_eq!(
        sent,
        vec!["Sent Messages".to_owned()],
        "a role names one folder; the server's own claim beats a look-alike"
    );
}

/// iCloud's shape: the provider's own Sent folder beside one another client
/// made, and nothing declared, so the alphabet would pick `Sent`.
async fn a_silent_server_with_two_sent_folders() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent"))
        .mailbox(MockMailbox::new("Sent Messages"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

fn sent_path(connection: &Connection, account: &Account) -> Option<String> {
    MailboxRepository::new(connection)
        .by_role(account.id, MailboxRole::Sent)
        .expect("by role")
        .map(|mailbox| mailbox.path)
}

#[tokio::test]
async fn an_accounts_own_map_outranks_the_configuration() {
    // ADR 0025: `[mailboxes]` is one table for every account; the account's
    // own map, in the store, is what the user said about *this* server.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_silent_server_with_two_sent_folders().await;

    let configured = RoleOverrides::from_pairs([(MailboxRole::Sent, "Sent")]);
    MailboxRoleRepository::new(&connection)
        .set(account.id, MailboxRole::Sent, "Sent Messages")
        .expect("the account's own choice");

    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("discover");

    assert_eq!(
        sent_path(&connection, &account).as_deref(),
        Some("Sent Messages"),
        "the account's map wins over [mailboxes]"
    );
    let look_alike = MailboxRepository::new(&connection)
        .by_path(account.id, "Sent")
        .expect("by path")
        .expect("the row");
    assert_eq!(
        look_alike.role,
        MailboxRole::Regular,
        "and the folder the configuration named is an ordinary folder here"
    );
}

#[tokio::test]
async fn an_accounts_map_says_nothing_about_another_account() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let icloud = an_account(&connection);
    let mut other = Account::new(
        "Other",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.net"),
    );
    AccountRepository::new(&connection)
        .create(&mut other)
        .expect("create account");
    let backend = a_silent_server_with_two_sent_folders().await;

    MailboxRoleRepository::new(&connection)
        .set(icloud.id, MailboxRole::Sent, "Sent Messages")
        .expect("map one account");

    for account in [&icloud, &other] {
        discover(&connection, &backend, account.id, &RoleOverrides::default())
            .await
            .expect("discover");
    }

    assert_eq!(
        sent_path(&connection, &icloud).as_deref(),
        Some("Sent Messages")
    );
    assert_eq!(
        sent_path(&connection, &other).as_deref(),
        Some("Sent"),
        "the other account resolves on its own, by the automatic rule"
    );
}

#[tokio::test]
async fn a_map_changed_between_passes_takes_effect_on_the_next() {
    // Nothing is frozen at startup: the engine's part is the configuration
    // tier only, and the account's map is read every pass, so a choice made
    // in settings needs no restart to be honoured by discovery.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection);
    let backend = a_silent_server_with_two_sent_folders().await;
    let configured = RoleOverrides::default();

    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("first pass");
    assert_eq!(sent_path(&connection, &account).as_deref(), Some("Sent"));

    MailboxRoleRepository::new(&connection)
        .set(account.id, MailboxRole::Sent, "Sent Messages")
        .expect("choose in settings");
    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("second pass");

    assert_eq!(
        sent_path(&connection, &account).as_deref(),
        Some("Sent Messages"),
        "the second pass honours the choice with the engine untouched"
    );
    let roles: Vec<String> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .expect("list")
        .into_iter()
        .filter(|mailbox| mailbox.role == MailboxRole::Sent)
        .map(|mailbox| mailbox.path)
        .collect();
    assert_eq!(
        roles,
        vec!["Sent Messages".to_owned()],
        "and still one folder per role"
    );
}
