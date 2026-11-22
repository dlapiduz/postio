//! POSTIO-MEASUREMENT: its output is numbers a person reads, and it costs
//! 44.1 s, so it runs on the nightly timer rather than the merge path
//! The idle engine's CPU must not follow store size: a per-tick scan is invisible at eleven rows
//! (#1216).

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MailboxEvent, MockBackend, MockMailbox};
use postio_core::bridge::event_channel;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::seed::seed_large;
use postio_storage::{BlobStore, test_support};
use postio_test_support::cpu::{assert_the_clock_can_see_a_burn, cpu_time};

const SIZES: [usize; 3] = [100, 5_000, 50_000];

/// Spans several 5 s `POLL_INTERVAL` ticks; a per-tick scan is what is sought.
const WINDOW: Duration = Duration::from_secs(5);

/// Loose: the evidence is the slope across `SIZES`; this only catches every reading being equally
/// terrible.
const CEILING: f64 = 0.05;

/// A scan follows row count; the slack is for the clock's 10 ms tick and a shared machine, not a
/// slope.
const SLOPE: f64 = 4.0;

#[tokio::test]
async fn an_idle_engine_costs_the_same_whatever_the_store_holds() {
    assert_the_clock_can_see_a_burn();

    let mut readings = Vec::new();
    for messages in SIZES {
        let (burned, elapsed, woke) = idle_for(messages, WINDOW).await;
        assert!(
            woke,
            "the engine did not answer a delivery after idling {elapsed:?} over \
             {messages} messages, so this measured a stopped engine rather than \
             an idling one"
        );
        let share = burned.as_secs_f64() / elapsed.as_secs_f64();
        eprintln!(
            "STORE {messages:>6} messages -> {burned:?} of CPU across {elapsed:?} \
             ({:.2}% of a core)",
            share * 100.0
        );
        assert!(
            share < CEILING,
            "an idle engine over {messages} messages burned {burned:?} in \
             {elapsed:?} — {:.1}% of a core. Idling is not work.",
            share * 100.0
        );
        readings.push(burned);
    }

    // The clock resolves 10 ms, so a floor keeps the ratio meaningful.
    let floor = Duration::from_millis(20);
    let smallest = readings.first().copied().expect("a reading").max(floor);
    let largest = readings.last().copied().expect("a reading").max(floor);
    let slope = largest.as_secs_f64() / smallest.as_secs_f64();
    assert!(
        slope <= SLOPE,
        "idling over {} messages cost {largest:?} against {smallest:?} over {} \
         — {slope:.1}x. Cost that follows the row count is a scan on the idle \
         path, which is what #1239 fixed once already.",
        SIZES[SIZES.len() - 1],
        SIZES[0],
    );
}

/// The flag says whether the engine still answered afterwards: a dead engine idles perfectly. It is
/// a delivery (`push_event` ends a held `IDLE`), not a count of calls, since a healthy engine
/// parked in `IDLE` makes none.
async fn idle_for(messages: usize, window: Duration) -> (Duration, Duration, bool) {
    let database = test_support::memory().await;
    let report = seed_large(&database, 11, messages).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(MockMailbox::new("Sent"))
            .build(),
    );

    let _engine = Engine::spawn(EngineParts {
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

    // The first sync is real work; measuring across it would measure the sync.
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(5)));

    let before = cpu_time();
    let started = Instant::now();
    std::thread::sleep(postio_test_support::scaled(window));
    let burned = cpu_time().saturating_sub(before);
    let elapsed = started.elapsed();

    let called_before = backend.calls();
    backend.push_event("INBOX", MailboxEvent::Exists { count: 1 });
    let give_up = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    let mut woke = false;
    while Instant::now() < give_up && !woke {
        woke = backend.calls() > called_before;
        if !woke {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    (burned, elapsed, woke)
}
