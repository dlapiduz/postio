//! The `<100 ms` search budget, measured while the body index is being
//! written — the condition every other bench in this workspace avoids.
//!
//! # Why this exists (#500)
//!
//! A search that replayed at 15 ms took 3.8 s in the application. The
//! difference was everything the idle benches never see: the body catch-up
//! pass writing continuously against the same store, on a real filesystem,
//! competing for the same page cache. `search_budget.rs` builds its corpus
//! with `test_support::memory()`, which lives on `/dev/shm` — WAL exists but
//! disk I/O does not, so no amount of write pressure there can slow a read.
//!
//! This bench is the missing condition: a **file-backed** corpus under
//! `CARGO_TARGET_TMPDIR` (real disk, not tmpfs), a writer thread running the
//! body catch-up's own write pattern — batched transactions behind the write
//! gate, a breather between batches — and searches timed on the main thread
//! while it runs. The budget holds because WAL keeps readers off the write
//! lock and the batching keeps the churn bounded; if either regresses — a
//! write pattern that starves readers, a transaction held across blob-scale
//! I/O — this is the bench that goes red.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-index --bench search_under_load
//! ```
//!
//! The budget is asserted on the **median** of a run of searches rather than
//! a single draw: this bench shares a disk with whatever else the machine is
//! doing, and one descheduled read must not fail a build the way a shifted
//! median should.

#![allow(missing_docs)]

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{AccountId, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::{Database, WritePriority, test_support};

/// docs/PRODUCT.md §18 / CLAUDE.md: local search must resolve in under this —
/// and "while the index is catching up" is not an exemption.
const SEARCH_BUDGET: Duration = Duration::from_millis(100);

/// Smaller than `search_budget.rs`'s corpus because every row here costs real
/// disk; still large enough that a starved read shows up as tens of
/// milliseconds rather than noise.
const MESSAGE_COUNT: u64 = 40_000;

/// One message in a hundred carries this word.
const UNCOMMON_WORD: &str = "quarterly";
/// Every message's body carries this word.
const COMMON_WORD: &str = "regarding";

/// The write pattern `postio_session::index_local_bodies` settled on after
/// #500: one transaction per batch of this many bodies, taken behind a
/// background write-gate permit, with a pause between batches.
const WRITER_BATCH: usize = 200;
const WRITER_BREATHER: Duration = Duration::from_millis(25);

/// How many timed searches make one measurement.
const DRAWS: usize = 15;

struct Corpus {
    database: Database,
    account_id: AccountId,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(build_corpus)
}

struct Xorshift64(u64);

impl Xorshift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// A file-backed corpus on real disk: under `CARGO_TARGET_TMPDIR` — inside
/// `target/`, which is on a real filesystem — never `/tmp`, which on the
/// reference platform is tmpfs and would quietly turn this back into the
/// bench that cannot see I/O. The directory is disposable with the rest of
/// `target/`, so nothing cleans it up.
fn build_corpus() -> Corpus {
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("search-under-load-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a bench scratch directory");
    let database = Database::open(directory.join("postio.db")).expect("a bench store");
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    connection.execute_batch("BEGIN").expect("begin bulk load");
    for i in 0..MESSAGE_COUNT {
        let mut message = Message::new(
            account.id,
            mailbox,
            base + chrono::Duration::minutes(i as i64),
        );
        message.from = vec![EmailAddress::new(
            Some(format!("Sender {}", i % 500)),
            format!("sender{}@example.com", i % 500),
        )];
        message.subject = Some(format!("Weekly update {i}"));
        message.size = 1024 + rng.below(4096);
        repository.create(&mut message).expect("create message");
        postio_index::index::index_body(&connection, message.id.get(), Some(&body_text(i)))
            .expect("index body");
    }
    connection
        .execute_batch("COMMIT")
        .expect("commit bulk load");
    drop(connection);

    Corpus {
        database,
        account_id: account.id,
    }
}

/// A few hundred words of body, the common word in all of them and the
/// uncommon one in every hundredth.
fn body_text(i: u64) -> String {
    let mut body = format!("{COMMON_WORD} the status as of message {i} ");
    let mut rng = Xorshift64(i.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    for _ in 0..120 {
        let word = rng.below(3000);
        body.push_str(&format!("w{word} "));
    }
    if i.is_multiple_of(100) {
        body.push_str(UNCOMMON_WORD);
    }
    body
}

/// Runs the catch-up's write pattern against `database` until told to stop:
/// re-index a batch of bodies in one gated transaction, breathe, repeat.
fn churn(database: Database, stop: &AtomicBool) {
    let mut rng = Xorshift64(0xc0ff_ee00_dead_beef);
    while !stop.load(Ordering::Relaxed) {
        let connection = database.connection().expect("writer checkout");
        {
            let _permit = connection.write_gate().acquire(WritePriority::Background);
            connection
                .execute_batch("BEGIN IMMEDIATE")
                .expect("writer begin");
            for _ in 0..WRITER_BATCH {
                let id = rng.below(MESSAGE_COUNT) as i64 + 1;
                postio_index::index::index_body(&connection, id, Some(&body_text(id as u64)))
                    .expect("writer index");
            }
            connection.execute_batch("COMMIT").expect("writer commit");
        }
        drop(connection);
        std::thread::sleep(WRITER_BREATHER);
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 6, 1, 0, 0, 0).unwrap()
}

/// One timed search, on a fresh page-cache-warm connection from the pool.
fn one_search(query: &str) -> Duration {
    let corpus = corpus();
    let connection = corpus.database.connection().expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(corpus.account_id),
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    let start = Instant::now();
    let results = search(&connection, &request, now()).expect("search");
    let elapsed = start.elapsed();
    assert!(!results.hits.is_empty(), "query {query:?} matched nothing");
    elapsed
}

/// The median of [`DRAWS`] searches run while the writer churns.
fn median_under_load(query: &str) -> Duration {
    let corpus = corpus();
    let stop = AtomicBool::new(false);
    let mut draws = Vec::with_capacity(DRAWS);
    std::thread::scope(|scope| {
        let database: &Database = &corpus.database;
        scope.spawn(|| churn(database.clone(), &stop));
        // Let the writer actually get going before the first draw.
        std::thread::sleep(Duration::from_millis(50));
        for _ in 0..DRAWS {
            draws.push(one_search(query));
            std::thread::sleep(Duration::from_millis(20));
        }
        stop.store(true, Ordering::Relaxed);
    });
    draws.sort();
    draws[DRAWS / 2]
}

fn assert_budget(name: &str, elapsed: Duration) {
    assert!(
        elapsed < SEARCH_BUDGET,
        "{name} took a median {elapsed:?} under write load, over the {SEARCH_BUDGET:?} budget"
    );
}

fn bench_simple_term_under_load(c: &mut Criterion) {
    c.bench_function("search_simple_term_under_load", |b| {
        b.iter(|| one_search(UNCOMMON_WORD))
    });
    assert_budget("simple term", median_under_load(UNCOMMON_WORD));
}

fn bench_common_word_under_load(c: &mut Criterion) {
    c.bench_function("search_common_word_under_load", |b| {
        b.iter(|| one_search(COMMON_WORD))
    });
    assert_budget("common-word worst case", median_under_load(COMMON_WORD));
}

criterion_group!(
    name = under_load;
    config = Criterion::default().sample_size(10).measurement_time(Duration::from_secs(8));
    targets = bench_simple_term_under_load, bench_common_word_under_load
);
criterion_main!(under_load);
