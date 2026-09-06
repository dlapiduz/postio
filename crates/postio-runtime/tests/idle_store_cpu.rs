//! What an idle engine costs, and whether the store's size changes it (#1216).
//!
//! # The suspect
//!
//! The one profile #1216 has is 6,000 samples, 92.67% on the sync thread, and
//! its top frames are SQLCipher and SQLite rather than anything to do with the
//! network:
//!
//! ```text
//! 24.80%  sha512_block_data_order_avx2
//!  1.85%  aesni_cbc_encrypt
//!  1.49%  pcache1Fetch
//!  1.24%  sqlite3VdbeExec
//! ```
//!
//! That is page decryption and HMAC authentication under a btree traversal:
//! the shape of **a query that reads more of the store than it needs**, run
//! often. One such query was found and fixed — `wake_due_snoozes` ran every
//! `POLL_INTERVAL` and scanned every message in the account until #1239 gave
//! it a partial index. The question this case settles is whether another one
//! is left, because a scan is the one bug whose cost is invisible on a seeded
//! test store and ruinous on a real mailbox.
//!
//! # Why a sweep rather than a threshold
//!
//! A single reading cannot tell "cheap" from "cheap *here*": eleven messages
//! is what every other engine test holds, and a per-tick scan of eleven rows
//! costs nothing measurable no matter how wrong it is. What distinguishes a
//! scan from an indexed read is not the number, it is the **slope** — so this
//! measures the same idle engine over stores three orders of magnitude apart
//! and asserts the cost does not follow.
//!
//! Measured on this workstation, 5 s of idling each:
//!
//! ```text
//! STORE    100 messages -> 20ms of CPU (0.40% of a core)
//! STORE   5000 messages -> 10ms of CPU (0.20% of a core)
//! STORE  50000 messages -> 10ms of CPU (0.20% of a core)
//! ```
//!
//! Flat, and at the resolution of the clock. So the idle loop holds no scan,
//! and #1216's remaining burn is not this.
//!
//! # Its own binary, and why
//!
//! [`postio_test_support::cpu::cpu_time`] reads this **process's** CPU, so a
//! neighbour in the same binary is measured too. `runtime_suite` is one binary
//! over libtest's thread pool; this case cannot live there. `smtp_wait_cpu.rs`
//! is separate for the same reason and says so.
//!
//! Nothing here touches the network: the backend is `MockBackend`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MailboxEvent, MockBackend, MockMailbox};
use postio_core::bridge::event_channel;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::seed::seed_large;
use postio_storage::{BlobStore, test_support};
use postio_test_support::cpu::{assert_the_clock_can_see_a_burn, cpu_time};

/// Store sizes to compare, smallest first.
///
/// The top one is a real mailbox's order of magnitude rather than a test's,
/// which is the whole point: a scan hides at eleven rows.
const SIZES: [usize; 3] = [100, 5_000, 50_000];

/// How long each engine is watched for.
///
/// Long enough to contain several `POLL_INTERVAL` ticks (5 s each), because a
/// per-tick scan is what is being looked for and a window shorter than a tick
/// could miss every one of them.
const WINDOW: Duration = Duration::from_secs(5);

/// The ceiling any single reading must stay under, as a fraction of one core.
///
/// Deliberately loose. This case's evidence is the **slope** across `SIZES`,
/// and the absolute bound is only here to catch the case where every reading
/// is equally terrible — flat and ruinous is not a pass.
const CEILING: f64 = 0.05;

/// How much the largest store is allowed to cost over the smallest.
///
/// A scan's cost follows the row count, so 50,000 rows against 100 would be a
/// factor of hundreds. An indexed read is flat, and the slack here is for the
/// clock's 10 ms tick and the noise of a shared machine — not for a slope.
const SLOPE: f64 = 4.0;

#[test]
fn an_idle_engine_costs_the_same_whatever_the_store_holds() {
    assert_the_clock_can_see_a_burn();

    let mut readings = Vec::new();
    for messages in SIZES {
        let (burned, elapsed, woke) = idle_for(messages, WINDOW);
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

    // The clock's resolution is 10 ms, so a reading of zero and a reading of
    // one tick are the same measurement. Comparing them directly would make
    // the assertion a coin toss; the floor is what keeps the ratio meaningful.
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

/// Spawn a real engine over a store of `messages`, let it settle, and return
/// what it burned while doing nothing, for how long, and whether it was still
/// answering afterwards.
///
/// That last flag is what stops a reading of zero from being good news about a
/// dead engine: a spawn that failed to connect, or a loop that ended, idles
/// perfectly.
///
/// It is a *delivery*, not a count of calls made during the window, and the
/// difference is the whole point. A correctly idling engine is parked inside
/// `IDLE` with a command outstanding, so it makes no new calls at all — the
/// first version of this counted them and passed only because a 5 s window on
/// a fast machine still caught the tail of startup. Under CI's scaled window
/// the engine had long since settled, the count was zero, and a healthy engine
/// was reported as a stopped one. What proves it is alive is that it still
/// *answers*.
fn idle_for(messages: usize, window: Duration) -> (Duration, Duration, bool) {
    let database = test_support::memory();
    let report = seed_large(&database, 11, messages);
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

    // The first sync is real work the engine is supposed to be doing, and over
    // 50,000 rows it is not brief. Measuring across it would measure the sync.
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(5)));

    let before = cpu_time();
    let started = Instant::now();
    std::thread::sleep(postio_test_support::scaled(window));
    let burned = cpu_time().saturating_sub(before);
    let elapsed = started.elapsed();

    // Now poke it. `push_event` is what ends a held `IDLE`, so an engine that
    // is merely quiet answers and one whose loop has ended does not.
    let called_before = backend.calls();
    backend.push_event("INBOX", MailboxEvent::Exists { count: 1 });
    let give_up = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    let mut woke = false;
    while Instant::now() < give_up && !woke {
        woke = backend.calls() > called_before;
        if !woke {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    (burned, elapsed, woke)
}
