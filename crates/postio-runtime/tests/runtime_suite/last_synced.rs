//! A completed pass writes down when it happened (#1281).
//!
//! `mailboxes.last_synced_at` has been in the schema since migration 0001 and
//! both frontends' sidebars read it. **Nothing ever wrote it.** Every store in
//! the world held `NULL`, so a Postio with five thousand messages in it told
//! its user — truthfully, as far as the code went — that it had never synced.
//!
//! Found by running the macOS build against a real account and reading the
//! footer beside a folder holding 4,985 messages.

use std::sync::Arc;

use chrono::Utc;
use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::bridge::event_channel;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::MailboxRepository;
use postio_storage::{BlobStore, test_support};

fn server() -> MockBackend {
    let message = b"From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: the gate\r\n\
         Message-ID: <m-1@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
         \r\n\
         The gate closes at six.\r\n";
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::new(message.to_vec())))
        .build()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_has_synced_says_when() {
    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, "INBOX");
        (account, inbox)
    };

    // Before: nothing has synced, and the store says so honestly.
    {
        let connection = database.connection().expect("a connection");
        let before = MailboxRepository::new(&connection)
            .get(inbox.id)
            .expect("a read")
            .expect("the inbox");
        assert_eq!(before.last_synced_at, None, "nothing has synced yet");
    }

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: Arc::new(server()),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    let before = Utc::now();
    let summary = engine.sync(inbox.id).await.expect("a sync pass");
    assert!(
        summary.inserted > 0,
        "the fixture synced nothing: {summary:?}"
    );

    let connection = database.connection().expect("a connection");
    let after = MailboxRepository::new(&connection)
        .get(inbox.id)
        .expect("a read")
        .expect("the inbox");
    let recorded = after
        .last_synced_at
        .expect("a completed pass records when it happened");
    assert!(
        recorded >= before - chrono::Duration::seconds(2),
        "the recorded time is not this pass: {recorded}"
    );
    assert_eq!(
        after.counts.total, 1,
        "and recording it did not disturb the counts the triggers maintain"
    );
}
