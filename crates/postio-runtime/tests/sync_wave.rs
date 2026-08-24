//! A first sync works several mailboxes at once, and stays interruptible.
//!
//! `postio-0d9.7`. The engine used to take one mailbox off its queue, sync it
//! to completion, and only then look at the next. Almost all of a first sync's
//! wall clock is a round trip — measured against a live server, a batch of two
//! hundred headers spent an order of magnitude longer waiting for the server
//! than writing to SQLite — so one mailbox at a time left the connection pool
//! it was given almost entirely unused.
//!
//! # Why the assertion is about the server rather than a stopwatch
//!
//! "Faster" is what the issue asks for and a wall-clock assertion is the one
//! thing that cannot be trusted on a machine running four builds at once (see
//! CLAUDE.md). What actually distinguishes a concurrent sync from a sequential
//! one is whether the server ever has more than one request in front of it,
//! and [`MockBackend::peak_in_flight`] answers exactly that: it counts calls
//! across the stretch they spend waiting out the injected latency, so a
//! sequential caller cannot get above one however fast the machine is.

use std::sync::Arc;
use std::time::Duration;

use postio_core::bridge::event_channel;
use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource};
use postio_storage::repository::{
    AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
};
use postio_storage::{BlobStore, Database, test_support};

/// Long enough that two passes overlap for an unmistakable stretch, short
/// enough that a whole suite of these still runs in seconds.
const LATENCY: Duration = Duration::from_millis(40);

fn message(n: u32) -> Vec<u8> {
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
}

fn folder(path: &str, attributes: &[&str], messages: u32) -> MockMailbox {
    let mut mailbox = MockMailbox::new(path);
    if !attributes.is_empty() {
        mailbox = mailbox.attributes(attributes.iter().copied());
    }
    for n in 1..=messages {
        mailbox = mailbox.message(MockMessage::new(message(n)));
    }
    mailbox
}

/// An account with nothing synced yet, and an engine pointed at `backend`.
///
/// Nothing is seeded locally on purpose: discovery is what fills the folder
/// table, and a first sync is the case this file is about.
fn engine_over(backend: Arc<MockBackend>) -> (Database, Engine) {
    let database = test_support::memory();
    let account = {
        let connection = database.connection().expect("a connection");
        test_support::account(&connection)
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, _events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
    })
    .expect("the engine starts");
    (database, engine)
}

/// How many messages the store holds under the mailbox at `path`, and `None`
/// while that folder has not been discovered yet.
fn stored(database: &Database, path: &str) -> Option<u32> {
    let connection = database.connection().ok()?;
    let account = AccountRepository::new(&connection)
        .list()
        .ok()?
        .into_iter()
        .next()?;
    let mailbox = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .ok()?
        .into_iter()
        .find(|mailbox| mailbox.path == path)?;
    MessageRepository::new(&connection)
        .count(&ListQuery {
            scope: ListScope::Mailbox(mailbox.id),
            limit: 0,
            after: None,
        })
        .ok()
}

/// Waits for `condition`, or gives up and says what was true when it did.
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(30), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_sync_asks_about_several_mailboxes_at_once() {
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 40))
            .mailbox(folder("Sent Messages", &["\\Sent"], 40))
            .mailbox(folder("Archive", &["\\Archive"], 40))
            .mailbox(folder("Deleted Messages", &["\\Trash"], 40))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine) = engine_over(backend.clone());

    until("every folder to finish its first sync", || {
        ["INBOX", "Sent Messages", "Archive", "Deleted Messages"]
            .iter()
            .all(|path| stored(&database, path) == Some(40))
    })
    .await;

    assert!(
        backend.peak_in_flight() > 1,
        "the engine never had more than one request in front of the server, so \
         it is still syncing one mailbox at a time: peak {}",
        backend.peak_in_flight()
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inbox_finishes_while_a_large_archive_is_still_going() {
    // The composition `postio-0d9.7` asks for: concurrency must not cost
    // `postio-0d9.6`'s guarantee that INBOX is readable first. A lane taken by
    // an archive is a lane INBOX is *not* queued behind, so this should hold
    // more comfortably than it did sequentially — but only if the wave keeps
    // taking mailboxes off the front of a priority-ordered queue.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 2_000))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine) = engine_over(backend.clone());

    until("INBOX to finish", || stored(&database, "INBOX") == Some(10)).await;

    let archive = stored(&database, "Archive").unwrap_or(0);
    assert!(
        archive < 2_000,
        "INBOX is only readable early if it did not have to wait out the \
         archive: the archive was already complete at {archive} messages"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_is_served_without_waiting_out_the_wave_it_arrived_during() {
    // The constraint `postio-0d9.7` will not trade away: a wave is background
    // work, and the user asking for something must not queue behind it. The
    // engine used to check its inbox only *between* mailboxes, so a question
    // asked during a forty-thousand-message archive waited out the archive.
    //
    // The assertion is not a stopwatch: it is that the archive was still
    // unfinished when the answer came back. A wave that had to run to
    // completion first could not produce that.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 4_000))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine) = engine_over(backend.clone());

    // Wait until the wave is demonstrably under way rather than guessing at a
    // sleep: one committed batch means a pass is running.
    until("the archive to start arriving", || {
        stored(&database, "Archive").is_some_and(|count| count > 0)
    })
    .await;

    engine
        .backfill_progress()
        .await
        .expect("the engine answers while it is syncing");

    let archive = stored(&database, "Archive").unwrap_or(0);
    assert!(
        archive < 4_000,
        "the answer only came back after the whole archive had synced, so the \
         question waited out the wave rather than interrupting it"
    );

    // And nothing was dropped on the way: an interrupted pass is requeued and
    // resumes, so the archive still finishes.
    until("the archive to finish anyway", || {
        stored(&database, "Archive") == Some(4_000)
    })
    .await;
    drop(engine);
}
