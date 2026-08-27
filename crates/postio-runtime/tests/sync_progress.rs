//! The status line hears about a sync *while* it is running.
//!
//! `postio-qhz.5`: watching the first live sync, nothing in the application
//! said it was working. It had connected, listed fifteen folders and was
//! writing tens of thousands of messages, and the sidebar looked exactly like
//! an idle account.
//!
//! Everything needed was already built. `postio_sync::StatusTracker` shapes a
//! committed batch's [`Progress`] into a status and throttles it to 250 ms;
//! `postio-gtk`'s sidebar renders `syncing 30% · imap` from
//! [`Event::SyncProgress`]. The engine collected every batch's progress into a
//! `Vec` and folded it into the tracker *after the pass returned*, with one
//! `Utc::now()` shared by the whole loop — so the tracker's own throttle threw
//! all but the first away, and that one arrived at the end of a sync that
//! takes minutes. The user watching it learned nothing until it was over.
//!
//! # Why the assertion is about the server rather than the clock
//!
//! "During the pass" cannot be tested by whether `sync()` has returned: the
//! old code emitted its progress inside `sync()` too, just at the end of it.
//! What actually distinguishes the two is whether the status line moved while
//! the server was still being asked for mail. `MockBackend::calls` counts
//! that, so the test asserts the thing the bug was about: progress reached the
//! UI with fetching still to do.

use std::sync::Arc;
use std::time::Duration;

use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::{BlobStore, test_support};

/// Five batches of `postio_sync::initial::DEFAULT_BATCH_SIZE`.
const MESSAGES: u32 = 1_000;

/// Enough that the pass outlives the tracker's 250 ms throttle several times
/// over, so a fix that reports as it goes is visibly different from one that
/// reports at the end. Applies to every backend call, so a pass is roughly a
/// second — slow for a unit test and the point of this one.
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
        Event::SyncProgress { done, total, .. } => Some((*done, *total)),
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
    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, "INBOX");
        (account, inbox)
    };

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, events) = event_channel();
    let backend = Arc::new(server());
    backend.set_latency(LATENCY);

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_imap::auth::StoredPasswordSource::new(Arc::new(
            postio_imap::secret::MemorySecretStore::default(),
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

    // Watch the stream and the server together. What matters is not when the
    // report arrived but what the server had left to do when it did.
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
    // The tracker throttles to 250ms and the pass takes about a second, so a
    // handful. Anything approaching one per batch means the throttle stopped
    // being applied.
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
