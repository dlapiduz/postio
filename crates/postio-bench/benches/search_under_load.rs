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
use postio_storage::{Store, WritePriority, test_support};

/// Multi-threaded because a store read may reach `block_in_place`.
fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the benches")
        })
        .block_on(future)
}

const SEARCH_BUDGET: Duration = Duration::from_millis(100);

const MESSAGE_COUNT: u64 = 40_000;

const UNCOMMON_WORD: &str = "quarterly";
const COMMON_WORD: &str = "regarding";

/// The write pattern `index_local_bodies` uses: one transaction per batch behind a background
/// write-gate permit, with a pause between batches.
const WRITER_BATCH: usize = 200;
const WRITER_BREATHER: Duration = Duration::from_millis(25);

const DRAWS: usize = 15;

struct Corpus {
    database: Store,
    account_id: AccountId,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| on_runtime(build_corpus()))
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

/// File-backed under `CARGO_TARGET_TMPDIR`, never `/tmp`: tmpfs would hide the I/O this bench
/// exists to see.
async fn build_corpus() -> Corpus {
    let directory = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("search-under-load-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a bench scratch directory");
    // The fixed test key: every page read costs a decrypt, as in the application's encrypted store
    // (ADR 0014).
    let database = Store::open(directory.join("postio.db"), &test_support::key())
        .await
        .expect("a bench store");
    let connection = database.connect().await.expect("checkout");
    on_runtime(postio_index::index::ensure_schema(&connection)).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    on_runtime(connection.execute_batch("BEGIN")).expect("begin bulk load");
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
        repository
            .create(&mut message)
            .await
            .expect("create message");
        on_runtime(postio_index::index::index_body(
            &connection,
            message.id.get(),
            Some(&body_text(i)),
        ))
        .expect("index body");
    }
    on_runtime(connection.execute_batch("COMMIT")).expect("commit bulk load");
    drop(connection);

    Corpus {
        database,
        account_id: account.id,
    }
}

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

async fn churn(database: Store, stop: &AtomicBool) {
    let mut rng = Xorshift64(0xc0ff_ee00_dead_beef);
    while !stop.load(Ordering::Relaxed) {
        let connection = database.connect().await.expect("writer checkout");
        {
            let _permit = connection
                .write_gate()
                .acquire(WritePriority::Background)
                .await;
            on_runtime(connection.execute_batch("BEGIN IMMEDIATE")).expect("writer begin");
            for _ in 0..WRITER_BATCH {
                let id = rng.below(MESSAGE_COUNT) as i64 + 1;
                on_runtime(postio_index::index::index_body(
                    &connection,
                    id,
                    Some(&body_text(id as u64)),
                ))
                .expect("writer index");
            }
            on_runtime(connection.execute_batch("COMMIT")).expect("writer commit");
        }
        drop(connection);
        std::thread::sleep(WRITER_BREATHER);
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 6, 1, 0, 0, 0).unwrap()
}

async fn one_search(query: &str) -> Duration {
    let corpus = corpus();
    let connection = corpus.database.connect().await.expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        mailboxes: None,
        account: AccountScope::Account(corpus.account_id),
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    let start = Instant::now();
    let results = on_runtime(search(&connection, &request, now())).expect("search");
    let elapsed = start.elapsed();
    assert!(!results.hits.is_empty(), "query {query:?} matched nothing");
    elapsed
}

fn median_under_load(query: &str) -> Duration {
    let corpus = corpus();
    let stop = AtomicBool::new(false);
    let mut draws = Vec::with_capacity(DRAWS);
    std::thread::scope(|scope| {
        let database: &Store = &corpus.database;
        scope.spawn(|| on_runtime(churn(database.clone(), &stop)));
        // Let the writer actually get going before the first draw.
        std::thread::sleep(Duration::from_millis(50));
        for _ in 0..DRAWS {
            draws.push(on_runtime(one_search(query)));
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
        b.iter(|| on_runtime(one_search(UNCOMMON_WORD)))
    });
    assert_budget("simple term", median_under_load(UNCOMMON_WORD));
}

fn bench_common_word_under_load(c: &mut Criterion) {
    c.bench_function("search_common_word_under_load", |b| {
        b.iter(|| on_runtime(one_search(COMMON_WORD)))
    });
    assert_budget("common-word worst case", median_under_load(COMMON_WORD));
}

criterion_group!(
    name = under_load;
    config = Criterion::default().sample_size(10).measurement_time(Duration::from_secs(8));
    targets = bench_simple_term_under_load, bench_common_word_under_load
);
criterion_main!(under_load);
