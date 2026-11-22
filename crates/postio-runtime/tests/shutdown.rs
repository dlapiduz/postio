//! `Engine::stop()` keeps its promise while a backfill is pumping (#759).
//!
//! POSTIO-OWN-BINARY: it installs the global tracing subscriber.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{MailboxRepository, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

use postio_test_support::logs::Captured;

/// 150 ms a call makes draining take tens of seconds, far past the grace, so a `stop()` that waits
/// for the drain is caught.
fn server(messages: u32) -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: backlog {n}\r\n\
             Message-ID: <backlog-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             Bytes the backfill has still to carry home.\r\n"
        )
        .into_bytes()
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=messages {
        inbox = inbox.message(MockMessage::new(message(n)));
    }
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.set_latency(Duration::from_millis(150));
    backend
}

#[tokio::test]
async fn stop_returns_inside_the_grace_while_a_backfill_is_pumping() {
    let captured = Captured::default();
    // Global, not per-thread: the engine's thread would have no subscriber. Warnings only: at DEBUG
    // the
    // store's per-statement traces slowed the stop being measured (#2167).
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("this test binary runs one test and owns the subscriber");

    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = postio_core::bridge::event_channel();

    let backend = Arc::new(server(120));
    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
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

    let waited = Instant::now();
    while backend.body_fetches().len() < 3 {
        assert!(
            waited.elapsed() < Duration::from_secs(90),
            "the backfill never started pumping bodies"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let fetched_before = backend.body_fetches().len();
    let stopping = Instant::now();
    engine.stop();
    let took = stopping.elapsed();
    let fetched_after = backend.body_fetches().len() - fetched_before;

    // Nothing new starts after the stop; counted in calls, not seconds, since a loaded runner
    // stretched a correct stop past a 2 s bound.
    let on_the_wire = backend.peak_in_flight();
    assert!(
        fetched_after <= on_the_wire,
        "the backfill fetched {fetched_after} bodies after Engine::stop(), more than the \
         {on_the_wire} that can be on the wire at once; it did not hear the shutdown"
    );
    let log = captured.text();
    assert!(
        took < Duration::from_secs(5),
        "Engine::stop() took {took:?}, the whole grace; the backfill did not hear the \
         shutdown. The engine warned:\n{log}"
    );
    assert!(
        !log.contains("did not stop within"),
        "the engine warned about its own shutdown:\n{log}"
    );

    let connection = database.connect().await.expect("a connection");
    let mailboxes = MailboxRepository::new(&connection)
        .list_for_account(report.account.id)
        .await
        .expect("reading the account's folders");
    let mut remaining: usize = 0;
    for mailbox in &mailboxes {
        remaining += MessageRepository::new(&connection)
            .needing_backfill_from(mailbox.id, 500, 0)
            .await
            .map(|candidates| candidates.len())
            .unwrap_or(0);
    }
    assert!(
        remaining > 0,
        "stopping mid-backfill left nothing to resume; the stop came too late to prove anything"
    );
}
