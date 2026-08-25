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
use postio_storage::test_support::TempDatabase;
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
///
/// # File-backed, because this file is about concurrency
///
/// `test_support::memory()` is the usual choice and is the wrong one here.
/// An in-memory database is opened with SQLite's *shared cache*, which is a
/// different locking model from the WAL one Postio actually runs on: locks are
/// per-table, and a reader blocks a writer outright rather than the two
/// running side by side. The engine's runtime is
/// `Builder::new_current_thread`, so a pass that blocks waiting for one of
/// those locks blocks every other lane on the same thread with it, and the
/// wave this file is about stops overlapping — not because the engine stopped
/// running passes concurrently, but because the store underneath it was one
/// Postio never uses. See #79, where exactly this made three lanes serialise.
fn engine_over(backend: Arc<MockBackend>) -> (TempDatabase, Engine) {
    let database = test_support::temp();
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
        mailbox_roles: Default::default(),
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
///
/// The deadline is a *liveness* bound — it exists to turn a hang into a
/// failure with a name, and for nothing else. It is deliberately enormous:
/// this file used to give it 30 seconds, which quietly made it a performance
/// budget, and a GitHub runner (or this box with four sessions compiling)
/// walked straight through it while the code under test was fine (#122,
/// #125). Performance claims live in the benches; a genuinely hung test
/// costing three minutes once is cheaper than a flake costing a bisection
/// every month.
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(180), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

/// How many header fetches the server has served for the archive so far.
///
/// Per mailbox so the watcher's own post-sync INBOX polling cannot blur a
/// count: nothing but the wave fetches the archive.
fn archive_fetches(backend: &MockBackend) -> usize {
    backend
        .header_fetches()
        .iter()
        .filter(|mailbox| *mailbox == "Archive")
        .count()
}

/// Whether `log` shows INBOX served ahead of the archive's completion.
///
/// The property "INBOX was not queued behind the archive" is an *order* on
/// the server's own call log, not a stopwatch race: an engine that queues
/// INBOX behind the archive serves every archive fetch first, so INBOX's
/// first header fetch landing before the archive's last is exactly the
/// discriminator — under any scheduler, at any load. (The old form asserted
/// the archive was still incomplete at the moment a poll noticed INBOX was
/// done, which goes vacuous whenever the machine is slow enough for both to
/// finish — the 1-in-5 flake of #125.)
///
/// First-INBOX against last-archive on purpose: once the initial sync ends,
/// the watcher may fetch INBOX again on its own schedule, so INBOX's *last*
/// fetch is not the initial sync's; its *first* always is.
fn inbox_was_not_queued_behind_the_archive(log: &[String]) -> bool {
    let first_inbox = log.iter().position(|mailbox| mailbox == "INBOX");
    let last_archive = log.iter().rposition(|mailbox| mailbox == "Archive");
    match (first_inbox, last_archive) {
        (Some(inbox), Some(archive)) => inbox < archive,
        _ => false,
    }
}

#[test]
fn the_order_check_rejects_a_sequential_archive_first_engine() {
    // The regression this file's second test exists to catch, in miniature:
    // an engine that syncs the archive to completion before touching INBOX
    // produces exactly this log, and the check must call it out. This is the
    // "verify your tests can fail" half (CLAUDE.md) — injectable here, where
    // simulating a starved scheduler in the real engine is not.
    let starved: Vec<String> = std::iter::repeat_n("Archive".to_owned(), 50)
        .chain(std::iter::once("INBOX".to_owned()))
        .collect();
    assert!(!inbox_was_not_queued_behind_the_archive(&starved));

    let healthy: Vec<String> = ["Archive", "INBOX", "Archive", "Archive"]
        .map(str::to_owned)
        .to_vec();
    assert!(inbox_was_not_queued_behind_the_archive(&healthy));

    // A log that never saw one of the two is an answer to a different
    // question, never a pass.
    assert!(!inbox_was_not_queued_behind_the_archive(&[
        "Archive".to_owned()
    ]));
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

    // Liveness only: wait for the whole wave, then judge the order the
    // server actually saw. Waiting for both is what makes the assertion
    // non-vacuous — nothing about a slow machine can change a log that has
    // already been written.
    until("both folders to finish their first sync", || {
        stored(&database, "INBOX") == Some(10) && stored(&database, "Archive") == Some(2_000)
    })
    .await;

    let log = backend.header_fetches();
    assert!(
        inbox_was_not_queued_behind_the_archive(&log),
        "INBOX's first header fetch came after the archive's last, so INBOX \
         waited out the whole archive: {log:?}"
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
    // The assertion is causal, not a stopwatch: archive fetches must still
    // be *served after* the answer comes back. An engine that had to run the
    // wave to completion first answers only once the archive's fetch count
    // has stopped moving, so the two counts below come out equal — under any
    // scheduler, at any load. (The earlier form asserted the archive's row
    // count was below its total at the moment the answer returned, which a
    // machine slow enough to finish the archive inside that window turned
    // vacuous — the CI flake this test was known for, #122.)
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 2_000))
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
    let fetches_at_answer = archive_fetches(&backend);

    // And nothing was dropped on the way: an interrupted pass is requeued and
    // resumes, so the archive still finishes.
    until("the archive to finish anyway", || {
        stored(&database, "Archive") == Some(2_000)
    })
    .await;

    let fetches_in_the_end = archive_fetches(&backend);
    assert!(
        fetches_at_answer < fetches_in_the_end,
        "no archive fetch was served after the answer came back, so the \
         question waited out the wave rather than interrupting it \
         ({fetches_at_answer} of {fetches_in_the_end} archive fetches had \
         already happened)"
    );
    drop(engine);
}
