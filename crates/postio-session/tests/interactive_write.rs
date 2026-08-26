//! An archive keystroke does not queue behind a backfill. #425.
//!
//! # What went wrong
//!
//! `ARCHITECTURE.md` §1 promises that a mutation is a local write, an
//! enqueue, an event and a repaint, and that the UI never awaits the network.
//! The dispatch side of that held. What did not was the *write*: during a
//! first sync, archiving a message took **1.8 seconds** to reach SQLite.
//!
//! Neither of the two mechanisms first suspected was the one. The connection
//! pool was almost idle throughout — `Pool::get` returned in two microseconds
//! and never had fewer than one connection spare — so it was not exhaustion.
//! Nor was it one long transaction: a background batch held the write lock
//! for 90–200 ms, nowhere near 1.8 s.
//!
//! It was **starvation**. SQLite takes one writer at a time and settles a
//! collision with `busy_timeout`, which is a retry loop rather than a queue:
//! the loser sleeps, backing off up to a hundred milliseconds, and then races
//! everyone else again. Two sync lanes commit batches back to back with no
//! gap between one `COMMIT` and the next `BEGIN IMMEDIATE`, so the keystroke
//! woke, lost, slept longer, and lost again — for a second and a half.
//!
//! The measurement that settles it is that shortening the background
//! transactions does *not* help: cut to an eighth of their size, the same
//! write still took half a second, because the number of races to lose went up
//! as fast as each one got shorter. A queue with a priority in it is the only
//! thing that fixes this, and that is `postio_storage::WriteGate`.
//!
//! # Why the assertion counts messages rather than milliseconds
//!
//! The bug is a duration, but a wall-clock assertion is the one thing that
//! cannot be trusted on a machine running four builds at once (CLAUDE.md, and
//! #122/#125 where exactly that flaked). So this measures the wait in the
//! only unit that is invariant to machine speed: **how much backfill got
//! written while the keystroke waited.**
//!
//! That is the quantity the fix actually bounds. Starved, the archive waits
//! out batch after batch and thousands of messages land ahead of it. Gated, it
//! waits for the write units already in progress — one per sync lane — and
//! nothing more, however fast or slow the box is. A slow machine makes both
//! sides slow together and the ratio holds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_core::bridge::{EventStream, event_channel};
use postio_core::state::{AppState, SharedState};
use postio_core::{Command, MessageTarget};
use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource};
use postio_session::actions::Actions;
use postio_storage::repository::{
    AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
};
use postio_storage::test_support::TempDatabase;
use postio_storage::{BlobStore, Database, test_support};

/// Enough round-trip cost that the backfill is still going when the keystroke
/// lands, without making the test slow.
const LATENCY: Duration = Duration::from_millis(40);

/// The folder whose backfill is meant to be in the way. Big enough that it
/// cannot finish before the keystroke does.
const BULK: &str = "Lists";
const BULK_MESSAGES: u32 = 3_000;

/// The most backfill that may land while one archive waits.
///
/// The fix bounds this at the background write units already in progress —
/// one per sync lane. `initial::WRITE_UNIT` is 25 and `sync_lanes` is at most
/// `MAX_SYNC_LANES` of 3, so 75 is the structural ceiling; measured here it is
/// consistently 25, exactly one unit.
///
/// A hundred leaves room for the ceiling plus a lane that began a unit in the
/// instant before the keystroke registered, and is still fifteen times below
/// what starvation produced — the better part of two thousand — so it fails
/// long before the old behaviour comes back.
const MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR: u32 = 100;

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
/// File-backed, and that is load-bearing here for the reason `sync_wave.rs`
/// records: an in-memory database uses SQLite's shared cache, whose
/// table-level locking is a different model from the WAL one Postio runs on
/// and the one this file is about.
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
    })
    .expect("the engine starts");
    (database, engine)
}

/// The mailbox the store knows at `path`, once discovery has found it.
fn mailbox_at(database: &Database, path: &str) -> Option<postio_model::Mailbox> {
    let connection = database.connection().ok()?;
    let account = AccountRepository::new(&connection)
        .list()
        .ok()?
        .into_iter()
        .next()?;
    MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .ok()?
        .into_iter()
        .find(|mailbox| mailbox.path == path)
}

/// How many messages the store holds under `path`.
fn stored(database: &Database, path: &str) -> u32 {
    let Some(mailbox) = mailbox_at(database, path) else {
        return 0;
    };
    let Ok(connection) = database.connection() else {
        return 0;
    };
    MessageRepository::new(&connection)
        .count(&ListQuery {
            scope: ListScope::Mailbox(mailbox.id),
            limit: 0,
            after: None,
        })
        .unwrap_or(0)
}

/// Waits for `condition`, or gives up and says what was true when it did.
///
/// A liveness bound and nothing else — deliberately enormous, for the reason
/// `sync_wave.rs` sets out: a deadline small enough to be a performance budget
/// is a flake waiting for a loaded machine.
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(180), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_archive_keystroke_does_not_wait_for_the_backfill() {
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 20))
            .mailbox(folder("Archive", &["\\Archive"], 5))
            .mailbox(folder(BULK, &[], BULK_MESSAGES))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine) = engine_over(backend.clone());

    // The keystroke needs somewhere to act: a message in INBOX, and an
    // Archive folder to file it into. Both come from the sync, which is also
    // what puts the bulk folder's backfill in the way.
    until("INBOX and Archive to arrive", || {
        stored(&database, "INBOX") > 0 && mailbox_at(&database, "Archive").is_some()
    })
    .await;

    let inbox = mailbox_at(&database, "INBOX").expect("an INBOX");
    let archive = mailbox_at(&database, "Archive").expect("an Archive");
    let subject = {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: 1,
                after: None,
            })
            .expect("a page")
            .into_iter()
            .next()
            .expect("a message in the inbox")
    };

    // Mid-backfill, and demonstrably so: the bulk folder has begun arriving
    // and is nowhere near done.
    until("the bulk backfill to be under way", || {
        stored(&database, BULK) > 200
    })
    .await;

    // ── the keystroke ───────────────────────────────────────────────────
    let state = SharedState::default();
    let (quiet, _quiet_events): (_, EventStream) = event_channel();
    state.update(&quiet, |app: &mut AppState| app.open_mailbox(inbox.id));
    state.update(&quiet, |app: &mut AppState| {
        app.select(Vec::new(), Some(subject.id))
    });
    let actions = Actions::new((*database).clone(), state);
    let (events, _stream): (_, EventStream) = event_channel();

    let backfill_before = stored(&database, BULK);
    let started = Instant::now();
    actions
        .run(
            &Command::Archive {
                target: MessageTarget::Selection,
            },
            &events,
        )
        .expect("the archive");
    let took = started.elapsed();
    let backfill_after = stored(&database, BULK);

    // The write really happened, so none of the below is about a no-op.
    let landed = {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .get(subject.id)
            .expect("a read")
            .expect("the message")
            .mailbox_id
    };
    assert_eq!(landed, archive.id, "the archive did not actually file it");

    // Non-vacuity: if the backfill had finished there was nothing to wait
    // behind, and a pass would mean nothing.
    assert!(
        backfill_after < BULK_MESSAGES,
        "the bulk backfill finished before the keystroke was measured, so this \
         run never exercised the contention it is about ({backfill_after} of \
         {BULK_MESSAGES} stored)"
    );

    let waited_for = backfill_after.saturating_sub(backfill_before);
    eprintln!(
        "the archive waited for {waited_for} messages of backfill ({took:?}); \
         the bound is {MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR}"
    );
    assert!(
        waited_for <= MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR,
        "the archive waited for {waited_for} messages of backfill to be written \
         ahead of it ({took:?} of wall clock). The write gate is supposed to \
         bound that at one write unit per sync lane. Starved — which is what \
         #425 was — this number ran into the thousands."
    );

    drop(engine);
}
