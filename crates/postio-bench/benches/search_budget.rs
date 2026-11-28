//! The `<100 ms` search budget on 120,000 messages. A date range in the same WHERE once tempted the
//! planner into walking the range per hit (#1809), so a word plus a date is one of the shapes.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{AccountId, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::{Store, test_support};

/// Multi-threaded because a store read may reach `block_in_place`. Criterion's `iter` runs on a
/// thread with no ambient runtime, so `block_on` is safe here.
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

/// The corpora are built on first use inside `on_runtime`, where a nested `block_on` is refused; a
/// scoped thread is outside it.
fn outside_the_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(work).join().expect("the corpus builds"))
}

const SEARCH_BUDGET: Duration = Duration::from_millis(100);

const MESSAGE_COUNT: u64 = 120_000;

const UNCOMMON_WORD: &str = "quarterly";
const COMMON_WORD: &str = "regarding";
const SENDER_COUNT: u64 = 500;

/// Sized like a real mailbox's (#746): sender affinity's cost multiplies by it, and a near-empty
/// table hid a per-candidate scan.
const CONTACT_COUNT: u64 = 20_000;

struct Corpus {
    database: Store,
    account_id: AccountId,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| outside_the_runtime(|| on_runtime(build_corpus())))
}

struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

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

async fn build_corpus() -> Corpus {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64::new(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    // One transaction for the whole load: each `create` nests a SAVEPOINT in it, so 120,000 commits
    // become one.
    connection
        .execute_batch("BEGIN")
        .await
        .expect("start bulk load transaction");
    for i in 0..MESSAGE_COUNT {
        // Uncorrelated with `i % 100` (which places `UNCOMMON_WORD`): `i % SENDER_COUNT` would pin
        // each sender to one residue mod 100 and the composed query would match nothing. Message 0
        // is forced onto sender 42 so `from:sender42 quarterly` has at least one hit.
        let sender = if i == 0 { 42 } else { rng.below(SENDER_COUNT) };
        let received_at = base + chrono::Duration::minutes(i as i64);
        let mut message = Message::new(account.id, mailbox, received_at);
        message.from = vec![EmailAddress::new(
            Some(format!("Sender {sender}")),
            format!("sender{sender}@example.com"),
        )];
        message.subject = Some(format!("Weekly update {i}"));
        message.size = 1024 + rng.below(4096);
        repository
            .create(&mut message)
            .await
            .expect("create message");

        let mut body = format!("{COMMON_WORD} the status as of message {i}");
        if i % 100 == 0 {
            body.push_str(&format!(" {UNCOMMON_WORD} figures attached"));
        }
        postio_index::index::index_body(&connection, message.id.get(), Some(&body))
            .await
            .expect("index body");
    }
    // #746: a contacts table at real-mailbox scale; an empty table multiplies sender affinity's
    // cost by zero and hid a bug that took seconds on 18k contacts.
    {
        for i in 0..CONTACT_COUNT {
            let address = if i < SENDER_COUNT {
                format!("sender{i}@example.com")
            } else {
                format!("correspondent{i}@example.com")
            };
            let seen = rng.below(100) as i64;
            connection
                .execute(
                    "INSERT OR IGNORE INTO addresses (address, address_normalized)
                     VALUES (?1, ?1)",
                    [address.as_str()],
                )
                .await
                .expect("seed address");
            connection
                .execute(
                    "INSERT INTO contacts (times_seen, created_at, updated_at)
                     VALUES (?1, 0, 0)",
                    [seen],
                )
                .await
                .expect("seed contact");
            connection
                .execute(
                    "UPDATE addresses SET contact_id = (SELECT max(id) FROM contacts)
                      WHERE address_normalized = ?1",
                    [address.as_str()],
                )
                .await
                .expect("seed owner");
            connection
                .execute(
                    "INSERT OR IGNORE INTO contact_sightings (address_id, account_id, times_seen)
                     SELECT id, ?2, ?3 FROM addresses WHERE address_normalized = ?1",
                    (address.as_str(), account.id.get(), seen),
                )
                .await
                .expect("seed sighting");
        }
    }
    connection
        .execute_batch("COMMIT")
        .await
        .expect("commit bulk load transaction");

    drop(connection);
    Corpus {
        database,
        account_id: account.id,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 6, 1, 0, 0, 0).unwrap()
}

async fn run(query: &str, limit: u32) -> Duration {
    let corpus = corpus();
    let connection = corpus.database.connect().await.expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        mailboxes: None,
        account: AccountScope::Account(corpus.account_id),
        query: &parsed,
        scope: Scope::AllMail,
        limit,
        order: postio_search::ResultOrder::Relevance,
    };

    let start = Instant::now();
    let results = search(&connection, &request, now()).await.expect("search");
    let elapsed = start.elapsed();
    assert!(!results.hits.is_empty(), "query {query:?} matched nothing");
    elapsed
}

async fn run_facets(query: &str) -> Duration {
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
    let facets = postio_index::executor::facets(&connection, &request)
        .await
        .expect("facets");
    let elapsed = start.elapsed();
    assert!(
        facets.hits(Scope::AllMail) > 0,
        "query {query:?} matched nothing"
    );
    elapsed
}

fn assert_budget(name: &str, elapsed: Duration) {
    assert!(
        elapsed < SEARCH_BUDGET,
        "{name} took {elapsed:?}, over the {SEARCH_BUDGET:?} budget"
    );
}

fn bench_simple_term(c: &mut Criterion) {
    c.bench_function("search_simple_term", |b| {
        b.iter(|| on_runtime(run(UNCOMMON_WORD, 50)))
    });
    assert_budget("simple term", on_runtime(run(UNCOMMON_WORD, 50)));
}

fn bench_operator_only(c: &mut Criterion) {
    let query = "from:sender42";
    c.bench_function("search_operator_only", |b| {
        b.iter(|| on_runtime(run(query, 50)))
    });
    assert_budget("operator-only", on_runtime(run(query, 50)));
}

fn bench_composed(c: &mut Criterion) {
    let query = "from:sender42 quarterly";
    c.bench_function("search_composed", |b| b.iter(|| on_runtime(run(query, 50))));
    assert_budget("composed", on_runtime(run(query, 50)));
}

/// `after:` keeps about two thirds of the corpus, so a plan that walks the range once per hit reads
/// tens of millions of rows.
fn bench_word_after_date(c: &mut Criterion) {
    let query = "quarterly after:2020-02-01";
    c.bench_function("search_word_after_date", |b| {
        b.iter(|| on_runtime(run(query, 50)))
    });
    assert_budget("word plus after:", on_runtime(run(query, 50)));
}

fn bench_common_word_worst_case(c: &mut Criterion) {
    c.bench_function("search_common_word", |b| {
        b.iter(|| on_runtime(run(COMMON_WORD, 50)))
    });
    assert_budget("common-word worst case", on_runtime(run(COMMON_WORD, 50)));
}

// Unified search needs more than one account: with one, unified and scoped are the same query over
// the same rows (#452). A separate corpus keeps the single-account baselines comparable.

const ACCOUNTS: u64 = 4;
const PER_ACCOUNT: u64 = 30_000;

struct MultiAccount {
    database: Store,
    first: AccountId,
}

fn multi_account_corpus() -> &'static MultiAccount {
    static CORPUS: OnceLock<MultiAccount> = OnceLock::new();
    CORPUS.get_or_init(|| outside_the_runtime(|| on_runtime(build_multi_account_corpus())))
}

async fn build_multi_account_corpus() -> MultiAccount {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64::new(0x51de_9876_5432_10ab);
    let repository = MessageRepository::new(&connection);
    let mut first = None;

    connection
        .execute_batch("BEGIN")
        .await
        .expect("start bulk load");
    for a in 0..ACCOUNTS {
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        first.get_or_insert(account.id);
        for i in 0..PER_ACCOUNT {
            // Interleaved in time across accounts, so a unified recency order cannot be served from
            // one account's index.
            let received_at = base + chrono::Duration::minutes((i * ACCOUNTS + a) as i64);
            let mut message = Message::new(account.id, mailbox, received_at);
            message.from = vec![EmailAddress::new(
                Some(format!("Sender {}", rng.below(SENDER_COUNT))),
                format!("sender{}@example.com", rng.below(SENDER_COUNT)),
            )];
            message.subject = Some(format!("Weekly update {i}"));
            message.size = 1024 + rng.below(4096);
            repository
                .create(&mut message)
                .await
                .expect("create message");

            let mut body = format!("{COMMON_WORD} the status as of message {i}");
            if i % 100 == 0 {
                body.push_str(&format!(" {UNCOMMON_WORD} figures attached"));
            }
            postio_index::index::index_body(&connection, message.id.get(), Some(&body))
                .await
                .expect("index body");
        }
    }
    connection
        .execute_batch("COMMIT")
        .await
        .expect("commit bulk load");

    drop(connection);
    MultiAccount {
        database,
        first: first.expect("at least one account"),
    }
}

async fn run_multi_account(query: &str, account: AccountScope) -> Duration {
    let corpus = multi_account_corpus();
    let connection = corpus.database.connect().await.expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        mailboxes: None,
        account,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };

    let start = Instant::now();
    let results = search(&connection, &request, now()).await.expect("search");
    let elapsed = start.elapsed();
    assert!(!results.hits.is_empty(), "query {query:?} matched nothing");
    elapsed
}

/// A term in every message is past `RANK_BY_RELEVANCE_LIMIT`, so the executor orders by recency;
/// with no account named only `idx_messages_recency` can supply that order.
fn bench_unified_common_word(c: &mut Criterion) {
    c.bench_function("search_unified_common_word", |b| {
        b.iter(|| on_runtime(run_multi_account(COMMON_WORD, AccountScope::Unified)))
    });
    assert_budget(
        "unified, common-word worst case",
        on_runtime(run_multi_account(COMMON_WORD, AccountScope::Unified)),
    );
}

/// The control: fails with the unified case if the corpus itself got slower.
fn bench_account_scoped_common_word(c: &mut Criterion) {
    let scoped = AccountScope::Account(multi_account_corpus().first);
    c.bench_function("search_account_scoped_common_word", |b| {
        b.iter(|| on_runtime(run_multi_account(COMMON_WORD, scoped)))
    });
    assert_budget(
        "account-scoped, common-word worst case",
        on_runtime(run_multi_account(COMMON_WORD, scoped)),
    );
}

/// Bounded by `TOTAL_HITS_CAP` like the hit count; a facet panel that walked the corpus per
/// keystroke would leak the 100 ms budget.
fn bench_facets_worst_case(c: &mut Criterion) {
    c.bench_function("search_facets_common_word", |b| {
        b.iter(|| on_runtime(run_facets(COMMON_WORD)))
    });
    assert_budget(
        "facets, common-word worst case",
        on_runtime(run_facets(COMMON_WORD)),
    );
}

criterion_group!(
    benches,
    bench_simple_term,
    bench_operator_only,
    bench_composed,
    bench_word_after_date,
    bench_common_word_worst_case,
    bench_facets_worst_case,
    bench_unified_common_word,
    bench_account_scoped_common_word
);
criterion_main!(benches);
