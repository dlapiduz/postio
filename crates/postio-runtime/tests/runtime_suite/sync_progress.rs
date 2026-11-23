//! The status line hears about a sync while it runs: the assertion is on what the server had left
//! to do when progress arrived, not on `sync()` returning.

use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::{BlobStore, test_support};

const MESSAGES: u32 = 1_000;

/// Long enough that the pass outlives the tracker's 250 ms throttle several times, so reporting as
/// it goes differs from reporting at the end.
const LATENCY: Duration = Duration::from_millis(120);

fn server() -> MockBackend {
    let message = |n: u32| {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: message {n}\r\n\
             Message-ID: <m-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             Body {n}.\r\n"
        )
        .into_bytes()
    };
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=MESSAGES {
        inbox = inbox.message(MockMessage::new(message(n)));
    }
    MockBackend::builder().mailbox(inbox).build()
}

fn progress(event: &Event) -> Option<(u32, u32)> {
    match event {
        Event::SyncProgress { done, total, .. } if *total > 0 => Some((*done, *total)),
        _ => None,
    }
}

fn drain(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_sync_reports_progress_while_it_still_has_mail_to_fetch() {
    let database = test_support::memory().await;
    let (account, inbox) = {
        let connection = database.connect().await.expect("a connection");
        let account = test_support::account(&connection).await;
        let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
        (account, inbox)
    };

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();
    let backend = Arc::new(server());
    backend.set_latency(LATENCY);

    let engine = Engine::spawn(EngineParts {
        account: account.id,
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

    let pass = {
        let engine = engine.clone();
        let mailbox = inbox.id;
        tokio::spawn(async move { engine.sync(mailbox).await })
    };

    let mut reports: Vec<(u32, u32)> = Vec::new();
    let mut calls_at_first_report: Option<u64> = None;
    while !pass.is_finished() {
        for event in drain(&events) {
            if let Some(report) = progress(&event) {
                calls_at_first_report.get_or_insert_with(|| backend.calls());
                reports.push(report);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let summary = pass.await.expect("the pass runs").expect("a sync pass");
    for event in drain(&events) {
        if let Some(report) = progress(&event) {
            reports.push(report);
        }
    }

    assert!(
        summary.inserted >= MESSAGES as usize,
        "the fixture did not actually sync a large mailbox: {summary:?}"
    );

    let calls_at_first_report = calls_at_first_report.expect(
        "no progress reached the UI at all while the pass was running; a sync \
         that takes minutes would look identical to an idle account",
    );
    assert!(
        calls_at_first_report < backend.calls(),
        "the first progress report arrived after call {calls_at_first_report} \
         of {}, so the status line only moved once there was nothing left to \
         fetch",
        backend.calls()
    );
    assert!(
        reports.len() > 1,
        "one report is a status line that moved once: {reports:?}"
    );
    // The tracker throttles to 250 ms; anything near one per batch means the throttle stopped
    // applying.
    assert!(
        reports.len() <= 5,
        "{} reports for five batches is close enough to per-batch that the \
         throttle is not doing anything: {reports:?}",
        reports.len()
    );
    assert!(
        reports.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "progress went backwards: {reports:?}"
    );
}
