//! What a first sync leaves resident, under the application's allocator
//! (#1606).
//!
//! A live instance held 908-917 MB resident after a first sync of a large
//! account, and one mimalloc arena held 624-671 MB of it -- 567 MB of that as
//! transparent huge pages. The review attributed it, from code and `/proc`,
//! to per-connection page caches (addressed by #1602), mimalloc's huge pages
//! and purge delay, and the engine's full-text segment cache. None of those
//! was measured. This measures what a first sync of one large folder leaves
//! behind, through the real engine and store against the mock server, with
//! the application's own allocator, so the allocator's knobs can be compared
//! by environment:
//!
//! ```text
//! cargo nextest run -p postio-runtime --test first_sync_memory --profile nightly --no-capture
//! MIMALLOC_ALLOW_THP=0 cargo nextest run ... (same)
//! ```
//!
//! Three samples: before the engine starts, when the folder's headers and
//! bodies are all local, and after a quiet spell for the allocator to purge.
//! Each reports RSS, the process peak, and the largest anonymous mapping --
//! the mimalloc arena -- with how much of it is huge pages.
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads and compares
//! across allocator settings, and a first sync of this size takes a minute.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::bridge::event_channel;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::{BlobStore, Store, test_support};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Messages in the folder: enough that the page caches, the index and the
/// allocator all reach a steady state, few enough to run in a minute.
const MESSAGES: u32 = 20_000;

/// `POSTIO_MEASURE_MESSAGES` overrides [`MESSAGES`] for a quicker look.
fn messages() -> u32 {
    std::env::var("POSTIO_MEASURE_MESSAGES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MESSAGES)
}

/// How long the process is left alone before the last sample.
const QUIET: Duration = Duration::from_secs(10);

fn message(n: u32) -> Vec<u8> {
    let body = "The tide gate interlock report for this week, with the \
                readings from every station along the channel. "
        .repeat(20);
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: interlock report {n}\r\n\
         Message-ID: <m-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         {body}\r\n"
    )
    .into_bytes()
}

#[derive(Debug)]
struct Sample {
    rss_kb: u64,
    peak_kb: u64,
    arena_rss_kb: u64,
    arena_thp_kb: u64,
}

fn sample() -> Sample {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    // The largest anonymous mapping, which in a mimalloc process is the arena.
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap_or_default();
    let (mut arena_rss_kb, mut arena_thp_kb) = (0u64, 0u64);
    let (mut anonymous, mut rss, mut thp) = (false, 0u64, 0u64);
    let mut close = |anonymous: bool, rss: u64, thp: u64| {
        if anonymous && rss > arena_rss_kb {
            arena_rss_kb = rss;
            arena_thp_kb = thp;
        }
    };
    for line in smaps.lines() {
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or_default();
        if first.contains('-') && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            close(anonymous, rss, thp);
            // Six fields and no path: an anonymous mapping.
            anonymous = line.split_whitespace().count() == 5;
            rss = 0;
            thp = 0;
        } else if first == "Rss:" {
            rss = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if first == "AnonHugePages:" {
            thp = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    close(anonymous, rss, thp);
    Sample {
        rss_kb: field("VmRSS:"),
        peak_kb: field("VmHWM:"),
        arena_rss_kb,
        arena_thp_kb,
    }
}

async fn local(database: &Store, sql: &str) -> i64 {
    let Ok(connection) = database.connect().await else {
        return -1;
    };
    postio_storage::sql::one(&connection, sql, (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .unwrap_or(-1)
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_first_sync_leaves_resident() {
    let mut archive = MockMailbox::new("Archive").attributes(["\\Archive"]);
    for n in 1..=messages() {
        archive = archive.message(MockMessage::new(message(n)));
    }
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(archive)
            .build(),
    );
    backend.refuse_creates("no new folders here");

    let database = test_support::temp().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        test_support::account(&connection).await
    };
    // `POSTIO_MEASURE_WITHOUT_FTS=1` drops the header full-text index before
    // the sync, so a second run says what the engine's segment cache holds:
    // the difference between the two is its share (#1606).
    let without_fts = std::env::var_os("POSTIO_MEASURE_WITHOUT_FTS").is_some();
    if without_fts {
        let connection = database.connect().await.expect("a connection");
        connection
            .execute("DROP INDEX IF EXISTS search_documents_fts", ())
            .await
            .expect("the index drops");
    }
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store");
    let (sink, _events) = event_channel();

    let before = sample();
    let started = Instant::now();
    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
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

    let mut headers_at: Option<Duration> = None;
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(1200));
    loop {
        let stored = local(&database, "SELECT count(*) FROM messages").await;
        let owed = local(
            &database,
            "SELECT count(*) FROM messages WHERE body_state IN ('not_fetched', 'headers_only')",
        )
        .await;
        if headers_at.is_none() && stored >= i64::from(messages()) {
            headers_at = Some(started.elapsed());
        }
        if stored >= i64::from(messages()) && owed == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the first sync did not finish: {stored} stored, {owed} bodies owed"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let synced = sample();
    let took = started.elapsed();

    // POSTIO-FIXED-DEADLINE: the quiet spell is the subject -- how much the
    // allocator gives back in that time -- not a wait for something to happen.
    tokio::time::sleep(QUIET).await;
    let quiet = sample();
    drop(engine);

    let thp = std::env::var("MIMALLOC_ALLOW_THP").unwrap_or_else(|_| "default".to_owned());
    let thp = if without_fts {
        format!("{thp}, no fts")
    } else {
        thp
    };
    for (when, at) in [("before", &before), ("synced", &synced), ("quiet", &quiet)] {
        eprintln!(
            "first-sync memory (MIMALLOC_ALLOW_THP={thp}) {when:>6}: rss {:>7} kB, \
             peak {:>7} kB, arena {:>7} kB of which huge pages {:>7} kB",
            at.rss_kb, at.peak_kb, at.arena_rss_kb, at.arena_thp_kb
        );
    }
    eprintln!(
        "first-sync memory (MIMALLOC_ALLOW_THP={thp}): {} messages in {took:.1?}, headers all local at {:.1?}",
        messages(),
        headers_at.unwrap_or_default()
    );

    // Only what holds on any machine: memory after a quiet spell is not above
    // the peak. The numbers are the point.
    assert!(quiet.rss_kb <= quiet.peak_kb);
}
