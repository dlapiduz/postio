use postio_core::Command;
use postio_core::bridge::event_channel;
use postio_core::state::SharedState;
use postio_model::{Mailbox, MailboxRole, Operation};
use postio_session::actions::Actions;
use postio_storage::repository::{MailboxRepository, OperationQueueRepository};
use postio_storage::test_support;

struct World {
    database: postio_storage::Store,
    account: postio_model::AccountId,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = test_support::account(&connection).await;
    let mut inbox = Mailbox::new(account.id, "INBOX", Some('/'));
    inbox.role = MailboxRole::Inbox;
    MailboxRepository::new(&connection)
        .create(&mut inbox)
        .await
        .expect("the inbox");
    World {
        database,
        account: account.id,
    }
}

impl World {
    async fn run(&self, command: Command) -> Result<(), String> {
        let (sink, _events) = event_channel();
        Actions::new(self.database.clone(), SharedState::default())
            .run(&command, &sink)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn create(
        &self,
        parent: Option<postio_model::MailboxId>,
        name: &str,
    ) -> Result<(), String> {
        self.run(Command::CreateFolder {
            account: Some(self.account),
            parent,
            name: Some(name.to_owned()),
        })
        .await
    }

    async fn folder(&self, path: &str) -> Option<Mailbox> {
        let connection = self.database.connect().await.expect("a connection");
        MailboxRepository::new(&connection)
            .by_path(self.account, path)
            .await
            .expect("read")
    }

    async fn queued(&self) -> Vec<Operation> {
        let connection = self.database.connect().await.expect("a connection");
        OperationQueueRepository::new(&connection)
            .pending(self.account, chrono::Utc::now() + chrono::Duration::days(1))
            .await
            .expect("the queue")
            .into_iter()
            .map(|row| row.operation)
            .collect()
    }
}

#[tokio::test]
async fn a_new_folder_is_written_pending_and_its_create_is_queued() {
    let world = world().await;
    world.create(None, "Receipts").await.expect("created");
    let folder = world
        .folder("Receipts")
        .await
        .expect("the folder is there at once");
    assert!(folder.pending_create && folder.selectable);
    assert_eq!(folder.role, MailboxRole::Regular);
    assert_eq!(
        world.queued().await,
        vec![Operation::CreateMailbox { mailbox: folder.id }]
    );
}

#[tokio::test]
async fn a_name_like_a_role_stays_an_ordinary_folder_under_a_parent() {
    let world = world().await;
    world.create(None, "Projects").await.expect("the parent");
    let parent = world.folder("Projects").await.expect("parent").id;
    world
        .create(Some(parent), "Archive")
        .await
        .expect("a child named like a role");
    let child = world
        .folder("Projects/Archive")
        .await
        .expect("the parent's delimiter joins them");
    assert_eq!(
        child.role,
        MailboxRole::Regular,
        "a name never becomes a role (ADR 0050)"
    );
    assert_eq!(child.parent_id, Some(parent));
}

#[tokio::test]
async fn names_that_cannot_be_a_new_folder_are_refused() {
    let world = world().await;
    world.create(None, "Receipts").await.expect("created");
    for (name, why) in [
        ("   ", "an empty name"),
        ("receipts", "a folder that exists, whatever the case"),
        ("inbox", "the inbox's name"),
        ("Trash", "a top-level role's name"),
        ("a/b", "a delimiter inside one name"),
    ] {
        assert!(
            world.create(None, name).await.is_err(),
            "{why} was accepted"
        );
    }
    assert_eq!(world.queued().await.len(), 1, "a refusal queues nothing");
}

#[tokio::test]
async fn discarding_a_pending_folder_takes_its_create_away() {
    let world = world().await;
    world.create(None, "Receipts").await.expect("created");
    let folder = world.folder("Receipts").await.expect("the folder").id;
    world
        .run(Command::DiscardFolder { mailbox: folder })
        .await
        .expect("undone");
    assert!(world.folder("Receipts").await.is_none());
    assert!(
        world.queued().await.is_empty(),
        "the create went with its row"
    );
}
