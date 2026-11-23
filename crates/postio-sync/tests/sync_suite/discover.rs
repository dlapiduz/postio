use postio_account::backend::{MailBackend, MockBackend, MockMailbox};
use postio_model::{Account, EmailAddress, MailboxRole, Message, RoleOverrides};
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, MailboxRepository, MailboxRoleRepository, MessageRepository,
};
use postio_storage::test_support;
use postio_sync::discover::discover;

async fn an_account(connection: &Connection) -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create account");
    account
}

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

async fn paths(connection: &Connection, account: &Account) -> Vec<String> {
    let mut paths: Vec<String> = MailboxRepository::new(connection)
        .list_for_account(account.id)
        .await
        .expect("list")
        .into_iter()
        .map(|mailbox| mailbox.path)
        .collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn discovery_writes_the_servers_folders_into_the_local_table() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    let report = discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert_eq!(report.added, 6, "{report:?}");
    assert_eq!(
        paths(&connection, &account).await,
        vec![
            "Archive",
            "Deleted Messages",
            "Drafts",
            "INBOX",
            "Junk",
            "Sent Messages"
        ]
    );
    assert_eq!(report.known(), 6, "every folder just added is known now");
    assert!(report.changed(), "a first discovery is never a no-op");
}

#[tokio::test]
async fn a_folders_role_comes_from_the_servers_attributes_not_its_name() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
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
            .await
            .expect("by role")
            .unwrap_or_else(|| panic!("no folder resolved to {role:?}"));
        assert_eq!(found.path, path);
    }
}

#[tokio::test]
async fn discovering_twice_keeps_the_same_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");
    let before: Vec<_> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .expect("list")
        .into_iter()
        .map(|mailbox| (mailbox.path, mailbox.id))
        .collect();

    let report = discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    let after: Vec<_> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .expect("list")
        .into_iter()
        .map(|mailbox| (mailbox.path, mailbox.id))
        .collect();

    assert_eq!(report.added, 0, "{report:?}");
    assert_eq!(
        before, after,
        "the ids everything else points at must not move"
    );
    assert_eq!(
        report.known(),
        6,
        "the account still has the same six folders the second time: the four \
         `a_server()` lists plus the Drafts and Junk the first pass created \
         because that server has neither (spec 003 FR-026). `added == 0` above \
         is what says the second pass created nothing."
    );
}

#[tokio::test]
async fn discovery_preserves_what_a_sync_pass_recorded() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let mailboxes = MailboxRepository::new(&connection);
    let mut inbox = mailboxes
        .by_role(account.id, MailboxRole::Inbox)
        .await
        .expect("by role")
        .expect("an inbox");
    inbox.generation = Some(postio_model::Generation::new(42));
    inbox.highest_mod_seq = Some(postio_model::ModSeq::new(900));
    mailboxes
        .update(&inbox)
        .await
        .expect("record a synced inbox");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    let after = mailboxes
        .get(inbox.id)
        .await
        .expect("get")
        .expect("still there");
    assert_eq!(after.generation, Some(postio_model::Generation::new(42)));
    assert_eq!(after.highest_mod_seq, Some(postio_model::ModSeq::new(900)));
}

#[tokio::test]
async fn discovery_does_not_reset_a_folders_backfill_exclusion() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");

    let mailboxes = MailboxRepository::new(&connection);
    let inbox = mailboxes
        .by_role(account.id, MailboxRole::Inbox)
        .await
        .expect("by role")
        .expect("an inbox");
    mailboxes
        .set_backfill_excluded(inbox.id, true)
        .await
        .expect("exclude the inbox");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    assert!(
        mailboxes.backfill_excluded(inbox.id).await.expect("read"),
        "a resync must not reach a decision the server was never asked about"
    );
}

#[tokio::test]
async fn a_folder_the_server_no_longer_lists_keeps_its_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");
    let mailboxes = MailboxRepository::new(&connection);
    let archive = mailboxes
        .by_role(account.id, MailboxRole::Archive)
        .await
        .expect("by role")
        .expect("an archive");

    let mut message = Message::new(account.id, archive.id, chrono::Utc::now());
    message.subject = Some("Filed away years ago".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("file a message in it");

    let smaller = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent Messages").attributes(["\\Sent"]))
        .build();
    smaller.connect().await.expect("connect");

    let report = discover(&connection, &smaller, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert_eq!(report.vanished, 4, "{report:?}");
    let after = mailboxes
        .get(archive.id)
        .await
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
            .await
            .expect("get")
            .is_some(),
        "and the mail in it survives"
    );
}

#[tokio::test]
async fn a_folder_that_comes_back_is_usable_again() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

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
        .await
        .expect("by role")
        .expect("an inbox");
    assert!(inbox.selectable);
}

#[tokio::test]
async fn an_empty_listing_is_not_read_as_every_folder_being_gone() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
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
    assert_eq!(paths(&connection, &account).await.len(), 6);
}

#[tokio::test]
async fn a_child_folder_is_linked_to_its_parent() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

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
        .await
        .expect("by path")
        .expect("an inbox");
    let child = mailboxes
        .by_path(account.id, "INBOX/Receipts")
        .await
        .expect("by path")
        .expect("the child");

    assert_eq!(child.parent_id, Some(inbox.id));

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("a second discovery");
    let child_again = mailboxes
        .by_path(account.id, "INBOX/Receipts")
        .await
        .expect("by path")
        .expect("the child");
    assert_eq!(
        child_again.id, child.id,
        "an already-linked child keeps its row"
    );
    assert_eq!(child_again.parent_id, Some(inbox.id));
    assert_eq!(child.name, "Receipts", "the leaf, not the whole path");
}

#[tokio::test]
async fn a_missing_intermediate_level_still_leaves_the_leaf_usable() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").delimiter('/'))
        .mailbox(MockMailbox::new("INBOX/Archive/2026").delimiter('/'))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    let mailboxes = MailboxRepository::new(&connection);
    let leaf = mailboxes
        .by_path(account.id, "INBOX/Archive/2026")
        .await
        .expect("by path")
        .expect("the leaf, listed even though its parent was never named");
    assert_eq!(
        leaf.parent_id, None,
        "with no local row for INBOX/Archive to point at, the leaf has no \
         parent rather than a fabricated one"
    );
}

#[tokio::test]
async fn a_folder_that_cannot_hold_messages_is_recorded_as_such() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

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
        .await
        .expect("by path")
        .expect("the folder");
    assert!(!lists.selectable);
}

async fn an_unhelpful_server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Vecchia Posta"))
        .mailbox(MockMailbox::new("Cestino"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

async fn role_of(connection: &Connection, account: &Account, path: &str) -> Option<MailboxRole> {
    MailboxRepository::new(connection)
        .list_for_account(account.id)
        .await
        .expect("list")
        .into_iter()
        .find(|mailbox| mailbox.path == path)
        .map(|mailbox| mailbox.role)
}

#[tokio::test]
async fn an_override_gives_a_role_to_a_folder_nothing_else_could_name() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = an_unhelpful_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");
    assert_eq!(
        role_of(&connection, &account, "Vecchia Posta").await,
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
        role_of(&connection, &account, "Vecchia Posta").await,
        Some(MailboxRole::Archive)
    );
    assert_eq!(
        role_of(&connection, &account, "Cestino").await,
        Some(MailboxRole::Trash)
    );
    assert_eq!(
        role_of(&connection, &account, "INBOX").await,
        Some(MailboxRole::Inbox),
        "the inbox is still the inbox; an override elsewhere does not disturb it"
    );
}

#[tokio::test]
async fn an_override_outranks_what_the_server_said() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_server().await;

    let overrides = RoleOverrides::from_pairs([(MailboxRole::Junk, "Archive")]);
    discover(&connection, &backend, account.id, &overrides)
        .await
        .expect("discover");

    assert_eq!(
        role_of(&connection, &account, "Archive").await,
        Some(MailboxRole::Junk)
    );
}

/// Changing a mapping re-labels folders and never moves mail: a role belongs to a mailbox row, and
/// moving mail on a config edit would be an unasked, irreversible IMAP move.
#[tokio::test]
async fn remapping_a_role_moves_the_label_and_never_the_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Archive").attributes(["\\Archive"]))
        .mailbox(MockMailbox::new("Vecchia Posta"))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    let archive = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .expect("list")
        .into_iter()
        .find(|mailbox| mailbox.path == "Archive")
        .expect("an Archive row");
    let mut message = Message::new(account.id, archive.id, chrono::Utc::now());
    message.subject = Some("filed under the old archive".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create message");

    let overrides = RoleOverrides::from_pairs([(MailboxRole::Archive, "Vecchia Posta")]);
    discover(&connection, &backend, account.id, &overrides)
        .await
        .expect("discover");

    assert_eq!(
        role_of(&connection, &account, "Vecchia Posta").await,
        Some(MailboxRole::Archive),
        "the new folder wears the role"
    );
    assert_eq!(
        role_of(&connection, &account, "Archive").await,
        Some(MailboxRole::Regular),
        "and the old one gives it up — two folders cannot both be the archive, \
         because `by_role` returns one"
    );

    let still_there = MessageRepository::new(&connection)
        .get(message.id)
        .await
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

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
        .await
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;

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
        .await
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

async fn a_silent_server_with_two_sent_folders() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Sent"))
        .mailbox(MockMailbox::new("Sent Messages"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

async fn sent_path(connection: &Connection, account: &Account) -> Option<String> {
    MailboxRepository::new(connection)
        .by_role(account.id, MailboxRole::Sent)
        .await
        .expect("by role")
        .map(|mailbox| mailbox.path)
}

#[tokio::test]
async fn an_accounts_own_map_outranks_the_configuration() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_silent_server_with_two_sent_folders().await;

    let configured = RoleOverrides::from_pairs([(MailboxRole::Sent, "Sent")]);
    MailboxRoleRepository::new(&connection)
        .set(account.id, MailboxRole::Sent, "Sent Messages")
        .await
        .expect("the account's own choice");

    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("discover");

    assert_eq!(
        sent_path(&connection, &account).await.as_deref(),
        Some("Sent Messages"),
        "the account's map wins over [mailboxes]"
    );
    let look_alike = MailboxRepository::new(&connection)
        .by_path(account.id, "Sent")
        .await
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let icloud = an_account(&connection).await;
    let mut other = Account::new(
        "Other",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.net"),
    );
    AccountRepository::new(&connection)
        .create(&mut other)
        .await
        .expect("create account");
    let backend = a_silent_server_with_two_sent_folders().await;

    MailboxRoleRepository::new(&connection)
        .set(icloud.id, MailboxRole::Sent, "Sent Messages")
        .await
        .expect("map one account");

    for account in [&icloud, &other] {
        discover(&connection, &backend, account.id, &RoleOverrides::default())
            .await
            .expect("discover");
    }

    assert_eq!(
        sent_path(&connection, &icloud).await.as_deref(),
        Some("Sent Messages")
    );
    assert_eq!(
        sent_path(&connection, &other).await.as_deref(),
        Some("Sent"),
        "the other account resolves on its own, by the automatic rule"
    );
}

#[tokio::test]
async fn a_map_changed_between_passes_takes_effect_on_the_next() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_silent_server_with_two_sent_folders().await;
    let configured = RoleOverrides::default();

    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("first pass");
    assert_eq!(
        sent_path(&connection, &account).await.as_deref(),
        Some("Sent")
    );

    MailboxRoleRepository::new(&connection)
        .set(account.id, MailboxRole::Sent, "Sent Messages")
        .await
        .expect("choose in settings");
    discover(&connection, &backend, account.id, &configured)
        .await
        .expect("second pass");

    assert_eq!(
        sent_path(&connection, &account).await.as_deref(),
        Some("Sent Messages"),
        "the second pass honours the choice with the engine untouched"
    );
    let roles: Vec<String> = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
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

async fn a_bare_server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

async fn roles_with_a_folder(connection: &Connection, account: &Account) -> Vec<MailboxRole> {
    let mailboxes = MailboxRepository::new(connection);
    let mut found = Vec::new();
    for role in MailboxRole::RESERVED {
        if mailboxes
            .by_role(account.id, role)
            .await
            .expect("by_role")
            .is_some()
        {
            found.push(role);
        }
    }
    found
}

#[tokio::test]
async fn a_server_with_only_an_inbox_gets_a_folder_for_every_reserved_role() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert_eq!(
        roles_with_a_folder(&connection, &account).await,
        MailboxRole::RESERVED.to_vec(),
        "an account whose server lists only INBOX still has to have somewhere \
         to archive, send, draft, delete and junk to"
    );
    let mut created = backend.created();
    created.sort();
    assert_eq!(created, vec!["Archive", "Drafts", "Junk", "Sent", "Trash"]);
}

#[tokio::test]
async fn a_second_pass_creates_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");
    let after_first = backend.created().len();

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    assert_eq!(
        backend.created().len(),
        after_first,
        "the second pass asked the server to create folders that already exist"
    );
}

#[tokio::test]
async fn a_server_that_already_has_everything_is_never_asked_to_create() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .mailbox(MockMailbox::new("Archive").attributes(["\\Archive"]))
        .mailbox(MockMailbox::new("Sent").attributes(["\\Sent"]))
        .mailbox(MockMailbox::new("Drafts").attributes(["\\Drafts"]))
        .mailbox(MockMailbox::new("Trash").attributes(["\\Trash"]))
        .mailbox(MockMailbox::new("Junk").attributes(["\\Junk"]))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert!(
        backend.created().is_empty(),
        "nothing was missing: {:?}",
        backend.created()
    );
}

#[tokio::test]
async fn the_inbox_is_never_created_even_when_the_server_does_not_list_one() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Archive").attributes(["\\Archive"]))
        .build();
    backend.connect().await.expect("connect");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert!(
        !backend.created().contains(&"INBOX".to_owned()),
        "the Inbox was created: {:?}",
        backend.created()
    );
}

#[tokio::test]
async fn a_server_that_refuses_leaves_the_role_unmapped_and_says_why() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;
    backend.refuse_creates("Permission denied");

    let report = discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("a refusal is not fatal to the pass");

    assert_eq!(
        roles_with_a_folder(&connection, &account).await,
        vec![MailboxRole::Inbox],
        "nothing was created, so only the folder the server actually has resolves"
    );
    assert!(
        report.known() > 0,
        "the pass still completed and the account is still usable: {report:?}"
    );

    let refusals = MailboxRoleRepository::new(&connection)
        .refusals(account.id)
        .await
        .expect("refusals");
    let roles: Vec<MailboxRole> = refusals.iter().map(|(role, _)| *role).collect();
    assert_eq!(
        roles,
        vec![
            MailboxRole::Archive,
            MailboxRole::Drafts,
            MailboxRole::Junk,
            MailboxRole::Sent,
            MailboxRole::Trash
        ],
        "every role that was refused is written down"
    );
    assert!(
        refusals
            .iter()
            .all(|(_, reason)| reason.contains("Permission denied")),
        "the server's own words are what tell somebody what to fix: {refusals:?}"
    );
}

#[tokio::test]
async fn a_refusal_is_not_retried_on_the_next_pass() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;
    backend.refuse_creates("Permission denied");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("first pass");
    let after_first = backend.created().len();
    assert_eq!(after_first, 5, "the first pass asks once per missing role");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("second pass");

    assert_eq!(
        backend.created().len(),
        after_first,
        "a server that will never allow a folder was asked again"
    );
}

#[tokio::test]
async fn a_refusal_for_one_role_does_not_stop_the_others() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;

    MailboxRoleRepository::new(&connection)
        .refuse(account.id, MailboxRole::Junk, "Permission denied")
        .await
        .expect("record a standing refusal for Junk alone");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("discover");

    assert_eq!(
        roles_with_a_folder(&connection, &account).await,
        vec![
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Trash
        ],
        "one refused role cost the others, or Junk was created anyway"
    );
    let mut created = backend.created();
    created.sort();
    assert_eq!(
        created,
        vec!["Archive", "Drafts", "Sent", "Trash"],
        "the standing refusal has to keep Junk out of the asking entirely"
    );
}

#[tokio::test]
async fn a_mailbox_that_already_exists_is_not_an_error() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account(&connection).await;
    let backend = a_bare_server().await;

    backend
        .create_mailbox("Archive")
        .await
        .expect("the folder appears");

    discover(&connection, &backend, account.id, &RoleOverrides::default())
        .await
        .expect("an existing folder is not a failure");

    assert_eq!(
        roles_with_a_folder(&connection, &account).await,
        MailboxRole::RESERVED.to_vec()
    );
    assert!(
        MailboxRoleRepository::new(&connection)
            .refusals(account.id)
            .await
            .expect("refusals")
            .is_empty(),
        "nothing was refused"
    );
}

#[tokio::test]
async fn a_created_folder_is_named_after_its_role_whatever_the_provider() {
    // FR-030: two entirely different servers must get identical folder names, since providers are
    // data and a provider-specific branch would break that.
    let names_for = |host: &'static str| async move {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let account = an_account(&connection).await;
        let backend = MockBackend::builder()
            .host(host)
            .mailbox(MockMailbox::new("INBOX"))
            .build();
        backend.connect().await.expect("connect");

        discover(&connection, &backend, account.id, &RoleOverrides::default())
            .await
            .expect("discover");
        let mut created = backend.created();
        created.sort();
        created
    };

    let one = names_for("imap.example.com").await;
    let other = names_for("imap.example.net").await;

    assert_eq!(one, vec!["Archive", "Drafts", "Junk", "Sent", "Trash"]);
    assert_eq!(
        one, other,
        "the folder names changed with the server, which is the branch \
         Principle VII forbids"
    );
}
