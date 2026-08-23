//! The search half of spec.md §18's `<100 ms` budget (CLAUDE.md), gated on a
//! corpus large enough to be honest about it: 120,000 messages, one account.
//!
//! Four query shapes, chosen to stress different parts of `search`:
//!
//! - **simple term** — a free-text word that matches about 1% of the corpus.
//!   The ordinary case: join `messages_fts`, rank the matches, snippet them.
//! - **operator-only** — `from:` with no free text at all. No `messages_fts`
//!   join; the structured-filter path and the SQL planner's use of
//!   `idx_messages_account_list`-adjacent indexes are what is under test.
//! - **composed** — an operator plus free text together, the shape the
//!   canvas 2b search bar produces on almost every real query.
//! - **common-word worst case** — a free-text word every message's body
//!   contains. `messages_fts MATCH` and the `count(*)` it feeds have to walk
//!   effectively the whole corpus; this is the shape most likely to blow the
//!   budget if an index is missing.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-search --bench search_budget --features index
//! ```
//!
//! Corpus generation is deterministic (a fixed-seed xorshift generator, no
//! wall-clock or OS randomness), so a run is reproducible byte-for-byte
//! across machines modulo timing. CI compiles this bench
//! (`cargo bench --workspace --no-run`) but does not time it — see
//! `postio-core/benches/perf_budgets.rs` for why a shared runner is not
//! trusted for millisecond budgets there either. Each `bench_*` function
//! below still asserts its own budget against a real `Instant` measurement,
//! so a genuine regression fails loudly here even outside CI.
//!
//! # Recording a new baseline
//!
//! ```sh
//! cargo bench -p postio-search --bench search_budget --features index -- --save-baseline main
//! ```

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use postio_model::{AccountId, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::{SearchRequest, parse, search};
use postio_storage::repository::MessageRepository;
use postio_storage::{Database, test_support};

/// spec.md §18 / CLAUDE.md: local search must resolve in under this.
const SEARCH_BUDGET: Duration = Duration::from_millis(100);

/// Large enough that an index gap shows up as milliseconds, not microseconds.
const MESSAGE_COUNT: u64 = 120_000;

/// One in a hundred messages carries this word — the "simple term" shape.
const UNCOMMON_WORD: &str = "quarterly";
/// Every message's body carries this word — the "common word" worst case.
const COMMON_WORD: &str = "regarding";
/// How many distinct senders the corpus spreads its messages across.
const SENDER_COUNT: u64 = 500;

struct Corpus {
    database: Database,
    account_id: AccountId,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(build_corpus)
}

/// A tiny, fixed-seed xorshift64 generator: reproducible across machines and
/// runs, and not worth pulling in a `rand` dependency for a bench fixture.
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

fn build_corpus() -> Corpus {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_search::index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64::new(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    // One transaction for the whole load: `MessageRepository::create` opens
    // its own SAVEPOINT per call (see `postio_storage::repository::Scope`),
    // which nests fine inside this outer transaction and turns 120,000
    // separate commits into one.
    connection
        .execute_batch("BEGIN")
        .expect("start bulk load transaction");
    for i in 0..MESSAGE_COUNT {
        // Uncorrelated with `i % 100` below (which places `UNCOMMON_WORD`)
        // on purpose: `i % SENDER_COUNT` would put every sender on a fixed
        // residue mod 100 since `SENDER_COUNT` divides evenly into 100's
        // multiples, so a sender chosen that way could never coincide with
        // the uncommon word and the "composed" query below would silently
        // match nothing.
        // Message 0 is forced onto sender 42 rather than left to the RNG, so
        // the "composed" query below (`from:sender42 quarterly`, and message
        // 0 gets `UNCOMMON_WORD` from `i % 100 == 0`) is guaranteed at least
        // one hit by construction rather than by a probabilistic coincidence.
        let sender = if i == 0 { 42 } else { rng.below(SENDER_COUNT) };
        let received_at = base + chrono::Duration::minutes(i as i64);
        let mut message = Message::new(account.id, mailbox, received_at);
        message.from = vec![EmailAddress::new(
            Some(format!("Sender {sender}")),
            format!("sender{sender}@example.com"),
        )];
        message.subject = Some(format!("Weekly update {i}"));
        message.size = 1024 + rng.below(4096);
        repository.create(&mut message).expect("create message");

        let mut body = format!("{COMMON_WORD} the status as of message {i}");
        if i % 100 == 0 {
            body.push_str(&format!(" {UNCOMMON_WORD} figures attached"));
        }
        postio_search::index::index_body(&connection, message.id.get(), Some(&body))
            .expect("index body");
    }
    connection
        .execute_batch("COMMIT")
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

fn run(query: &str, limit: u32) -> Duration {
    let corpus = corpus();
    let connection = corpus.database.connection().expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        account_id: corpus.account_id,
        query: &parsed,
        scope: Scope::AllMail,
        limit,
    };

    let start = Instant::now();
    let results = search(&connection, &request, now()).expect("search");
    let elapsed = start.elapsed();
    assert!(!results.hits.is_empty(), "query {query:?} matched nothing");
    elapsed
}

/// [`run`], but for the canvas' left column: the three scope counts and the
/// refine aggregates, measured together the way the panel asks for them.
fn run_facets(query: &str) -> Duration {
    let corpus = corpus();
    let connection = corpus.database.connection().expect("checkout");
    let parsed = parse(query, now().date_naive());
    let request = SearchRequest {
        account_id: corpus.account_id,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
    };

    let start = Instant::now();
    let facets = postio_search::executor::facets(&connection, &request).expect("facets");
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
    c.bench_function("search_simple_term", |b| b.iter(|| run(UNCOMMON_WORD, 50)));
    assert_budget("simple term", run(UNCOMMON_WORD, 50));
}

fn bench_operator_only(c: &mut Criterion) {
    let query = "from:sender42";
    c.bench_function("search_operator_only", |b| b.iter(|| run(query, 50)));
    assert_budget("operator-only", run(query, 50));
}

fn bench_composed(c: &mut Criterion) {
    let query = "from:sender42 quarterly";
    c.bench_function("search_composed", |b| b.iter(|| run(query, 50)));
    assert_budget("composed", run(query, 50));
}

fn bench_common_word_worst_case(c: &mut Criterion) {
    c.bench_function("search_common_word", |b| b.iter(|| run(COMMON_WORD, 50)));
    assert_budget("common-word worst case", run(COMMON_WORD, 50));
}

/// The facet column on the worst case the corpus has.
///
/// Four aggregate queries over a match set that is effectively the whole
/// corpus. They are bounded by `TOTAL_HITS_CAP` exactly as the hit count is,
/// which is what this asserts: a panel that could cost a full-corpus walk
/// per keystroke would be the one place the `<100 ms` budget quietly leaks.
fn bench_facets_worst_case(c: &mut Criterion) {
    c.bench_function("search_facets_common_word", |b| {
        b.iter(|| run_facets(COMMON_WORD))
    });
    assert_budget("facets, common-word worst case", run_facets(COMMON_WORD));
}

criterion_group!(
    benches,
    bench_simple_term,
    bench_operator_only,
    bench_composed,
    bench_common_word_worst_case,
    bench_facets_worst_case
);
criterion_main!(benches);
