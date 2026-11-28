use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_model::MailboxRole;
use postio_runtime::engine::{Clock, Engine, EngineParts, NetworkSource};
use postio_storage::{BlobStore, test_support};

const MESSAGES: u32 = 1_000;

/// Advances a fixed step per call, so the event count is a function of the batches committed, not
/// wall-clock time (#507).
struct FakeClock {
    base: Instant,
    step: Duration,
    calls: AtomicU32,
}

impl FakeClock {
    fn new(step: Duration) -> Self {
        FakeClock {
            base: Instant::now(),
            step,
            calls: AtomicU32::new(0),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.base + self.step * call
    }
}

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

fn drain(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_sync_tells_the_list_as_it_goes_and_not_once_per_batch() {
    let database = test_support::memory().await;
    let (account, inbox) = {
        let connection = database.connect().await.expect("a connection");
        let account = test_support::account(&connection).await;
        let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
        (account, inbox)
    };
    assert_eq!(inbox.role, MailboxRole::Inbox);

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
        clock: Arc::new(FakeClock::new(Duration::from_millis(200))),
    })
    .expect("the engine starts");

    let summary = engine.sync(inbox.id).await.expect("a sync pass");
    assert!(
        summary.inserted >= MESSAGES as usize,
        "the fixture did not actually sync a large mailbox: {summary:?}"
    );

    let told = drain(&events)
        .iter()
        .filter(
            |event| matches!(event, Event::MessageListChanged { mailbox, .. } if *mailbox == inbox.id),
        )
        .count();

    assert!(
        told > 1,
        "the list was told {told} time(s) — only at the end of the pass, so a \
         real first sync leaves it empty for as long as the sync takes"
    );
    // Two on any machine (see `FakeClock`); the range stays loose so a change to
    // `DEFAULT_BATCH_SIZE` fails with a comprehensible number.
    assert!(
        told <= 3,
        "the list was told {told} times for five batches; that is close enough \
         to per-batch to cost a reload each time"
    );
}
