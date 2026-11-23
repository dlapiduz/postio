use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{TestMailbox, TestMessage, TestServer};
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_core::{Command, Event, MessageTarget};
use postio_host::Host;
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_model::{
    Account, AccountId, EmailAddress, Flag, ListScope, MailboxId, MessageId, TransportSecurity,
};
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_storage::test_support;

const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

const DELIVERED_MESSAGE_ID: &str = "<harbour-dev.20260302T081200.a1@lists.example.org>";

const INBOX_PATH: &str = "INBOX";
const ARCHIVE_PATH: &str = "Archive";

fn patience() -> Duration {
    postio_test_support::scaled(Duration::from_secs(120))
}

fn eventually<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + patience();
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn account_for(server: &TestServer, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(None::<String>, address));
    account.incoming.host = server.addr().ip().to_string();
    account.incoming.port = server.addr().port();
    account.incoming.security = TransportSecurity::None;
    account.incoming.username = server.account().to_owned();
    account
}

struct World {
    rt: tokio::runtime::Runtime,
    host: Option<Host>,
    secrets: Arc<MemorySecretStore>,
    _blobs: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime for the server");
        let database = rt.block_on(test_support::memory());
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = postio_storage::BlobStore::open(
            directory.path().to_path_buf(),
            &test_support::blob_keys(),
        )
        .expect("a blob store");
        let secrets = Arc::new(MemorySecretStore::new());
        let host = Host::start(database, blobs, {
            let secrets = Arc::clone(&secrets);
            move |wiring| wiring.with_secrets(secrets)
        })
        .expect("a host");
        World {
            rt,
            host: Some(host),
            secrets,
            _blobs: directory,
        }
    }

    fn host(&self) -> &Host {
        self.host.as_ref().expect("running")
    }

    fn add(&self, mut account: Account, password: &str) -> Account {
        let database = self.host().wiring().database.clone();
        self.rt.block_on(async {
            let connection = database.connect().await.expect("a connection");
            AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("the account row");
            self.secrets
                .store(
                    &AccountKey::new(account.address.address.clone()),
                    &Password::new(password),
                )
                .await
                .expect("the memory store accepts a password");
        });
        account
    }

    fn mailbox(&self, account: AccountId, role: MailboxRole) -> Option<MailboxId> {
        let database = self.host().wiring().database.clone();
        self.rt.block_on(async {
            let connection = database.connect().await.ok()?;
            MailboxRepository::new(&connection)
                .by_role(account, role)
                .await
                .ok()
                .flatten()
                .map(|mailbox| mailbox.id)
        })
    }

    fn rows(&self, client: &Client, mailbox: MailboxId) -> Vec<MessageId> {
        let page = self.rt.block_on(client.list_page(PageRequest {
            scope: ListScope::Mailbox(mailbox),
            offset: 0,
            limit: 50,
        }));
        match page {
            Ok(ListPage::Messages(page)) => page.rows.into_iter().map(|row| row.id).collect(),
            Ok(ListPage::Threads(page)) => page
                .rows
                .into_iter()
                .map(|row| row.representative.id)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn id_of(&self, rfc_message_id: &str) -> Option<MessageId> {
        let database = self.host().wiring().database.clone();
        self.rt.block_on(async {
            let connection = database.connect().await.ok()?;
            postio_storage::sql::one(
                &connection,
                "SELECT id FROM messages WHERE rfc_message_id = ?1 AND deleted_locally = 0",
                postio_storage::bind![rfc_message_id],
                |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
            )
            .await
            .ok()
            .map(MessageId::new)
        })
    }

    fn uid_of(&self, message: MessageId) -> postio_model::Uid {
        let database = self.host().wiring().database.clone();
        self.rt.block_on(async {
            let connection = database.connect().await.expect("a connection");
            postio_storage::repository::MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("a read")
                .expect("the row is in the store")
                .server
                .uid
                .expect("a synced row carries its server uid")
        })
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            host.stop();
            drop(host);
        }
    }
}

#[test]
fn a_frontend_action_reaches_the_server_and_a_delivery_reaches_its_list() {
    let world = World::new();
    let server = world.rt.block_on(
        TestServer::builder()
            .account("test@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(SEEDED))
            .mailbox(TestMailbox::new("Archive").attributes(["\\Archive"]))
            .start(),
    );
    let account = world.add(account_for(&server, "Test", "test@example.com"), "hunter2");
    let client = world.host().connect(ClientKind::Test);
    let events = client.events();

    world.host().start_syncing();

    let inbox = eventually("the first sync made an inbox", || {
        world.mailbox(account.id, MailboxRole::Inbox)
    });
    let shown = eventually("the first sync's rows reached the list", || {
        let rows = world.rows(&client, inbox);
        (rows.len() == SEEDED.len()).then_some(rows)
    });
    let archive = eventually("the first sync made an archive", || {
        world.mailbox(account.id, MailboxRole::Archive)
    });
    assert!(
        server.uids(ARCHIVE_PATH).is_empty(),
        "the fixture's Archive starts empty, or archiving proves nothing"
    );

    let flagged = shown[0];
    let flagged_uid = world.uid_of(flagged);
    world
        .rt
        .block_on(client.send(Command::Flag {
            target: MessageTarget::Messages(vec![flagged]),
            flagged: Some(true),
        }))
        .expect("sent");
    eventually("the server's copy is \\Flagged", || {
        server
            .flags(INBOX_PATH, flagged_uid)
            .contains(&Flag::Flagged)
            .then_some(())
    });

    let archived = shown[1];
    let archived_uid = world.uid_of(archived);
    world
        .rt
        .block_on(client.send(Command::Archive {
            target: MessageTarget::Messages(vec![archived]),
        }))
        .expect("sent");
    eventually("the message reached the server's Archive", || {
        (!server.uids(ARCHIVE_PATH).is_empty()).then_some(())
    });
    assert!(
        !server.uids(INBOX_PATH).contains(&archived_uid),
        "the message reached Archive but was never taken out of INBOX: {:?}",
        server.uids(INBOX_PATH)
    );
    assert!(
        world.rows(&client, archive).contains(&archived)
            || world.rows(&client, inbox).iter().all(|id| *id != archived),
        "the archived row is still in the inbox list"
    );

    // Named by `Message-ID` rather than counted: the archive above may still be draining its local
    // half, so a total can be one off (#364).
    assert!(
        world.id_of(DELIVERED_MESSAGE_ID).is_none(),
        "the fixture the delivery sends is already in the store"
    );
    server.deliver(INBOX_PATH, TestMessage::corpus("list-thread-01-root"));
    let delivered = eventually("the delivery reached the inbox list", || {
        let id = world.id_of(DELIVERED_MESSAGE_ID)?;
        world.rows(&client, inbox).contains(&id).then_some(id)
    });
    let mut heard = false;
    while let Ok(envelope) = events.try_recv() {
        if matches!(
            envelope.event,
            Event::MessageListChanged { .. } | Event::MessagesChanged { .. }
        ) {
            heard = true;
        }
    }
    assert!(heard, "the client heard nothing about {delivered:?}");
}

#[test]
fn an_account_added_to_a_running_host_syncs_without_a_restart() {
    let world = World::new();
    let first = world.rt.block_on(
        TestServer::builder()
            .account("test@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
            .start(),
    );
    let joining = world.rt.block_on(
        TestServer::builder()
            .account("grace@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(["attachment-pdf"]))
            .start(),
    );
    let one = world.add(account_for(&first, "Test", "test@example.com"), "hunter2");
    let client = world.host().connect(ClientKind::Test);
    world.host().start_syncing();
    let inbox = eventually("the first account synced", || {
        let inbox = world.mailbox(one.id, MailboxRole::Inbox)?;
        (world.rows(&client, inbox).len() == 1).then_some(inbox)
    });

    let grace = world.add(
        account_for(&joining, "Grace", "grace@example.com"),
        "hunter2",
    );
    world.host().start_syncing();

    let grace_inbox = eventually("the joining account synced its mail", || {
        let inbox = world.mailbox(grace.id, MailboxRole::Inbox)?;
        (world.rows(&client, inbox).len() == 1).then_some(inbox)
    });
    assert_ne!(grace_inbox, inbox);
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    for address in ["test@example.com", "grace@example.com"] {
        assert!(
            accounts.iter().any(|a| a.address.address == address),
            "the client does not list {address}"
        );
    }
    assert!(
        !first.commands().is_empty() && !joining.commands().is_empty(),
        "both servers were spoken to"
    );
}
