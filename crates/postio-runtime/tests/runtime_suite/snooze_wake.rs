//! A snooze whose time has come wakes on its own while the app is open.
//!
//! #493: `MessageRepository::wake_due` clears a due snooze and says which
//! mailbox to repaint, but nothing calls it — visibility is self-healing
//! against wall-clock time on every list query regardless, so nothing failed
//! loudly when it went unwired. This drives the actual sync engine, the same
//! one every account gets, and proves its `POLL_INTERVAL` tick is what
//! reaches into the store and tells the list. Without that wiring, a message
//! due while the app is sitting open only reappears the next time something
//! else happens to touch the row.
//!
//! `NetworkSource::Ignored` and a `MockBackend` with nothing in it: nothing
//! here is about syncing, so the engine's own connect-and-poll loop is left
//! to do as little as it can while the ticker this test is about still runs
//! on the same loop.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use postio_account::backend::{MockBackend, MockMailbox};
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

fn drain(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_due_snooze_wakes_and_repaints_without_being_asked() {
    let database = test_support::memory();
    let (account, inbox, message_id) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        let message_id = MessageRepository::new(&connection)
            .create(&mut message)
            .expect("insert a message");
        MessageRepository::new(&connection)
            .snooze(&[message_id], Utc::now() - Duration::from_secs(1))
            .expect("snooze it into the past, so it is already due");
        (account, inbox, message_id)
    };

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: Arc::new(
            MockBackend::builder()
                .mailbox(MockMailbox::new("INBOX"))
                .build(),
        ),
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

    let deadline = std::time::Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    let mut told = false;
    while std::time::Instant::now() < deadline && !told {
        for event in drain(&events) {
            if let Event::MessageListChanged {
                account: a,
                mailbox,
            } = event
                && a == account.id
                && mailbox == inbox
            {
                told = true;
            }
        }
        if !told {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    assert!(
        told,
        "no MessageListChanged arrived for the mailbox holding the due message; \
         the engine's tick never woke it"
    );

    let connection = database.connection().expect("a connection");
    assert_eq!(
        MessageRepository::new(&connection)
            .get(message_id)
            .expect("a read")
            .expect("still there")
            .snoozed_until,
        None,
        "the sweep must clear the snooze, not just report it"
    );

    let _ = engine;
}
