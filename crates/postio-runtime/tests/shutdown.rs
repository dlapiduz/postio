//! `Engine::stop()` keeps its promise while a backfill is pumping (#759).
//!
//! Closing the job channel is the engine's only stop signal, and for a long
//! time three of the loop's phases could not hear it: the body-backfill
//! drains gated on `inbox.is_empty()` — and a **closed** channel is also
//! empty — while the sync wave's post-completion drain checked nothing at
//! all. Quitting mid-backfill then meant `stop()` burned the whole
//! [`SHUTDOWN_GRACE`], warned, and detached a thread that was still writing
//! SQLCipher pages while `exit()` tore libcrypto down underneath it — the
//! coredump `docs/engineering-notes.md` documents under #610/#300.
//!
//! One test, in a binary of its own: it asserts over everything the engine
//! logs, and a global subscriber cannot be shared with tests that log for
//! other reasons.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{MailboxRepository, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A writer every `tracing` line lands in, so the test can read them back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A server holding one large mailbox of small messages, every call slowed.
///
/// The latency is what makes the backfill long: at 150 ms a call, draining
/// this INBOX takes tens of seconds — far past the grace — so a `stop()`
/// that waits for the drain instead of interrupting it is caught red-handed
/// rather than racing the queue to empty.
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

#[test]
fn stop_returns_inside_the_grace_while_a_backfill_is_pumping() {
    let captured = Captured::default();
    // Globally, not per-thread: the engine works on a thread of its own, and
    // `set_default` would leave that thread — the one whose warning this
    // test exists to catch — with no subscriber at all.
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("this test binary runs one test and owns the subscriber");

    let database = test_support::memory();
    let report = seed_small(&database, 11);
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
        // Never dialled: nothing here queues a send.
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

    // Let the backfill genuinely start pumping bodies, so the stop below
    // lands mid-drain rather than before the queue was ever seeded.
    let waited = Instant::now();
    while backend.body_fetches().len() < 3 {
        assert!(
            waited.elapsed() < Duration::from_secs(90),
            "the backfill never started pumping bodies"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let stopping = Instant::now();
    engine.stop();
    let took = stopping.elapsed();

    // Well inside the 5 s grace: the loop notices the closed channel between
    // bodies and the in-flight fetch is cancelled, so what remains is the
    // tail of one call, not the rest of the queue.
    assert!(
        took < Duration::from_secs(2),
        "Engine::stop() took {took:?}; the backfill did not hear the shutdown"
    );
    let log = captured.text();
    assert!(
        !log.contains("did not stop within"),
        "the engine warned about its own shutdown:\n{log}"
    );

    // Nothing was lost with the queue: whatever was still unfetched — the
    // interrupted body included — is re-derivable from `body_state`, so the
    // next session's seed offers it again.
    let connection = database.connection().expect("a connection");
    let mailboxes = MailboxRepository::new(&connection)
        .list_for_account(report.account.id)
        .expect("reading the account's folders");
    let remaining: usize = mailboxes
        .iter()
        .map(|mailbox| {
            MessageRepository::new(&connection)
                .needing_backfill_from(mailbox.id, 500, 0)
                .map(|candidates| candidates.len())
                .unwrap_or(0)
        })
        .sum();
    assert!(
        remaining > 0,
        "stopping mid-backfill left nothing to resume; the stop came too late to prove anything"
    );
}
