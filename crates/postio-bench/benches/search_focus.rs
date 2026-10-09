//! Conversation search for Focus (spec 010), against its 50 ms budget.
//!
//! The results view asks the engine for a page of conversations with their
//! facets and months, then for the passages of that page, and prints how long
//! that took in its footer. So this measures the executor's calls
//! (`search_conversations`, `relaxation_counts`) and, as one more shape of its
//! own, that whole exchange as the session makes it: `conversations` for a
//! first page of 50 hits, then `passages` for those hits. That last number is
//! the one a person is shown.
//!
//! # The corpus
//!
//! 20,000 messages, deterministic (fixed-seed xorshift, no clock, no OS
//! randomness): threads of 1 to 12 messages, 40 senders with a Zipf spread,
//! 12 labels, 30% of messages with an attachment, spread over 24 months. The
//! plan also names 2,000 extracted attachment units; nothing reads attachment
//! contents yet (step 9, `contents_complete` is constant), so there is no
//! table to put them in and the corpus has none.
//!
//! # The shapes
//!
//! One word (about 1% of messages), two words, an operator alone, an operator
//! with words, a common word (most messages), zero hits with four filters
//! (the relaxation path), and what the search bar sends while `a`, `at`, `atl` are typed (the words as typed: the
//! dropdown's Conversations read; `completions` is step 8's, T115). Step 4
//! adds the filter popovers' live preview -- the same request asked again
//! per check, with a person, a label or a folder added or excluded -- and
//! the timeline's and the Date popover's dates, alone and with words.
//! Step 8 (T115) adds `completions` -- what the dropdown asks while `a`,
//! `at`, `atl` and `from:a` are typed -- against its own 20 ms budget
//! (SC-002), over the same corpus plus 2,000 contacts beside the 40
//! senders, some of whom the person has written to.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-bench --bench search_focus
//! ```
//!
//! It prints p50 and p95 per shape and the statement counts, then asserts
//! that every full-search shape's p95 is under [`BUDGET`]. Release, on the
//! development machine: a shared CI runner cannot defend a millisecond
//! budget, so CI compiles this (`cargo bench --workspace --no-run`) and
//! times nothing. The numbers are recorded in `docs/notes/`.

#![allow(missing_docs)]
// A bench is not public API; see `search_budget.rs`.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use postio_index::executor::{
    ConversationRequest, completions, relaxation_counts, search_conversations,
};
use postio_model::{
    Account, AccountScope, Attachment, EmailAddress, Flag, Label, Message, MessageId, Thread,
};
use postio_search::results::{ConversationOrder, Source};
use postio_search::{ParsedQuery, parse};
use postio_storage::repository::{LabelRepository, MessageRepository, ThreadRepository};
use postio_storage::test_support::{self, counting};
use postio_storage::{Checkout, Store};

/// The spec's budget (FR-062): a full search, facets included, over this
/// corpus, at the 95th percentile.
const BUDGET: Duration = Duration::from_millis(50);

/// Suggestions for a prefix (SC-002): what one keystroke of a short prefix
/// or an operator's value asks, at the 95th percentile.
const COMPLETIONS_BUDGET: Duration = Duration::from_millis(20);

/// Contacts beyond the senders: the address book a person who has mailed
/// for a while has.
const CONTACTS: u64 = 2_000;

const MESSAGES: u64 = 20_000;
const SENDERS: usize = 40;
const LABELS: usize = 12;
const MONTHS: i64 = 24;
/// Timed runs per shape, after [`WARMUP`] untimed ones.
const SAMPLES: usize = 60;
const WARMUP: usize = 5;

/// About 1% of messages carry this.
const ONE_WORD: &str = "quarterly";
/// About 5%.
const SECOND_WORD: &str = "forecast";
/// 95% of messages: "most".
const COMMON_WORD: &str = "regarding";

fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
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

/// A fixed-seed xorshift64: reproducible, and not worth a `rand` dependency.
struct Rng(u64);

impl Rng {
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

    /// True `per_mille` times in a thousand.
    fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }
}

/// The day the corpus ends with, and the clock the app would read.
fn today() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap()
}

struct Corpus {
    // Held so the in-memory store outlives the checkout.
    _store: Store,
    connection: Checkout,
    account: Account,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| on_runtime(build()))
}

/// Sender `k` of 40 with weight 1/(k+1): the first writes the most.
fn zipf_sender(rng: &mut Rng, cumulative: &[u64]) -> usize {
    let total = *cumulative.last().expect("senders");
    let pick = rng.below(total);
    cumulative.partition_point(|&c| c <= pick)
}

async fn build() -> Corpus {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mut rng = Rng(0x5eed_0010_f0c5_5ea7);
    let mut running = 0;
    let cumulative: Vec<u64> = (0..SENDERS)
        .map(|k| {
            running += 100_000 / (k as u64 + 1);
            running
        })
        .collect();

    let labels = LabelRepository::new(&connection);
    let mut label_ids = Vec::new();
    for k in 0..LABELS {
        let name = [
            "Atlas", "Harbor", "Q3 close", "Travel", "Receipts", "Legal", "Hiring",
        ]
        .get(k)
        .map_or_else(|| format!("Project {k}"), |name| (*name).to_owned());
        let mut label = Label::new(account.id, name);
        label_ids.push(labels.create(&mut label).await.expect("label"));
    }

    let messages = MessageRepository::new(&connection);
    let threads = ThreadRepository::new(&connection);
    let first = Utc.with_ymd_and_hms(2024, 10, 1, 0, 0, 0).unwrap();
    let span_minutes = MONTHS * 30 * 24 * 60;
    let fillers = [
        "agenda", "account", "attached", "atlas", "atlantic", "review", "invoice", "notes",
        "schedule", "reminder", "thanks", "update",
    ];

    connection.execute_batch("BEGIN").await.expect("begin");
    let mut made = 0;
    while made < MESSAGES {
        let size = (1 + rng.below(12)).min(MESSAGES - made);
        let sender = zipf_sender(&mut rng, &cumulative);
        let mut ids: Vec<MessageId> = Vec::new();
        for _ in 0..size {
            let received =
                first + chrono::Duration::minutes(made as i64 * span_minutes / MESSAGES as i64);
            let sender = if rng.chance(300) {
                zipf_sender(&mut rng, &cumulative)
            } else {
                sender
            };
            let mut message = Message::new(account.id, mailbox, received);
            message.from = vec![EmailAddress::new(
                Some(format!("Sender {sender}")),
                format!("sender{sender}@example.com"),
            )];
            message.to = vec![EmailAddress::new(Some("Me"), "me@example.com")];
            message.subject = Some(format!("Weekly update {made}"));
            if rng.chance(300) {
                let mut attachment =
                    Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2048);
                attachment.filename = Some(format!("report-{made}.pdf"));
                message.attachments.push(attachment);
            }
            if rng.chance(600) {
                message.flags.insert(Flag::Seen);
            }
            messages.create(&mut message).await.expect("message");

            let mut body = String::new();
            if !rng.chance(50) {
                body.push_str(COMMON_WORD);
                body.push(' ');
            }
            body.push_str(&format!("the status at a glance as of message {made}. "));
            for _ in 0..8 {
                body.push_str(fillers[rng.below(fillers.len() as u64) as usize]);
                body.push(' ');
            }
            if rng.chance(20) {
                body.push_str("atl office ");
            }
            if rng.chance(10) {
                body.push_str(&format!("{ONE_WORD} figures "));
            }
            if rng.chance(50) {
                body.push_str(&format!("{SECOND_WORD} numbers "));
            }
            postio_index::index::index_body(&connection, message.id.get(), Some(&body))
                .await
                .expect("body");
            connection
                .execute(
                    "UPDATE messages SET body_state = 'full' WHERE id = ?1",
                    [message.id.get()],
                )
                .await
                .expect("body present");
            for _ in 0..rng.below(3) {
                let label = label_ids[rng.below(LABELS as u64) as usize];
                labels.attach(message.id, label).await.expect("attach");
            }
            ids.push(message.id);
            made += 1;
        }
        let mut thread = Thread::new(account.id);
        let id = threads.create(&mut thread).await.expect("thread");
        for message in ids {
            threads.add_message(id, message).await.expect("join");
        }
    }
    // The address book: every sender, counted as the collector counts them,
    // and two thousand more people, a quarter of whom were written to.
    connection
        .execute(
            "INSERT INTO contacts (account_id, name, address, address_normalized,
                                   times_seen, last_seen_at)
             SELECT NULL, max(r.name), a.address, a.address_normalized, count(*),
                    max(m.received_at)
               FROM recipients r
               JOIN addresses a ON a.id = r.address_id
               JOIN messages m ON m.id = r.message_id
              WHERE r.kind = 'from'
              GROUP BY a.id",
            (),
        )
        .await
        .expect("the senders as contacts");
    let given = [
        "Ada", "Adam", "Adele", "Aiden", "Alex", "Amara", "Ben", "Bea", "Carmen", "Dmitri",
        "Elena", "Farid", "Grace", "Hana", "Ivo", "Juno", "Kai", "Lena", "Mateo", "Nia",
    ];
    for n in 0..CONTACTS {
        let name = format!(
            "{} Person{n}",
            given[rng.below(given.len() as u64) as usize]
        );
        let address = format!("person{n}@example.org");
        let seen = rng.below(60) as i64;
        let last = (first + chrono::Duration::days(rng.below(700) as i64)).timestamp_millis();
        connection
            .execute(
                "INSERT INTO contacts (account_id, name, address, address_normalized,
                                       times_seen, last_seen_at)
                 VALUES (NULL, ?1, ?2, ?2, ?3, ?4)",
                (name.as_str(), address.as_str(), seen, last),
            )
            .await
            .expect("a contact");
        if rng.chance(250) {
            connection
                .execute(
                    "INSERT INTO addresses (address, address_normalized) VALUES (?1, ?1)",
                    [address.as_str()],
                )
                .await
                .expect("an address");
            connection
                .execute(
                    "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
                     SELECT id, ?2, ?3 FROM addresses WHERE address_normalized = ?1",
                    (address.as_str(), 1 + rng.below(40) as i64, last),
                )
                .await
                .expect("a correspondent");
        }
    }
    connection.execute_batch("COMMIT").await.expect("commit");

    Corpus {
        _store: store,
        connection,
        account,
    }
}

/// One shape: a name, a query, and what one run of it does.
struct Shape {
    name: &'static str,
    query: &'static str,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `search_conversations`, first page of 50, facets and months included.
    Executor,
    /// The session's `conversations` then `passages`: the results view.
    EndToEnd,
    /// `relaxation_counts` over `relax(query)`, after a search that found nothing.
    Relaxations,
    /// `completions` for the prefix typed, or for `from:`'s value (step 8).
    Completions,
}

const SHAPES: &[Shape] = &[
    Shape {
        name: "one word",
        query: "quarterly",
        kind: Kind::Executor,
    },
    Shape {
        name: "two words",
        query: "quarterly forecast",
        kind: Kind::Executor,
    },
    Shape {
        name: "operator only",
        query: "from:sender3",
        kind: Kind::Executor,
    },
    Shape {
        name: "operator + words",
        query: "from:sender3 regarding",
        kind: Kind::Executor,
    },
    Shape {
        name: "common word",
        query: "regarding",
        kind: Kind::Executor,
    },
    Shape {
        name: "typed a",
        query: "a",
        kind: Kind::Executor,
    },
    Shape {
        name: "typed at",
        query: "at",
        kind: Kind::Executor,
    },
    Shape {
        name: "typed atl",
        query: "atl",
        kind: Kind::Executor,
    },
    Shape {
        name: "completions a",
        query: "a",
        kind: Kind::Completions,
    },
    Shape {
        name: "completions at",
        query: "at",
        kind: Kind::Completions,
    },
    Shape {
        name: "completions atl",
        query: "atl",
        kind: Kind::Completions,
    },
    Shape {
        name: "completions from:a",
        query: "from:a",
        kind: Kind::Completions,
    },
    Shape {
        name: "zero hits, four filters",
        query: "quarterly from:sender30 label:atlas has:attachment after:2025-01-01",
        kind: Kind::Executor,
    },
    Shape {
        name: "relaxations of the above",
        query: "quarterly from:sender30 label:atlas has:attachment after:2025-01-01",
        kind: Kind::Relaxations,
    },
    Shape {
        name: "e2e one word",
        query: "quarterly",
        kind: Kind::EndToEnd,
    },
    Shape {
        name: "e2e two words",
        query: "quarterly forecast",
        kind: Kind::EndToEnd,
    },
    Shape {
        name: "e2e operator + words",
        query: "from:sender3 regarding",
        kind: Kind::EndToEnd,
    },
    Shape {
        name: "e2e common word",
        query: "regarding",
        kind: Kind::EndToEnd,
    },
    Shape {
        name: "e2e typed atl",
        query: "atl",
        kind: Kind::EndToEnd,
    },
    // Step 4's live preview (T084): a popover's check asks the same
    // request again with one term added, per toggle, and Esc asks the
    // query it opened on once more; the timeline and the Date popover ask
    // the words with `after:` and `before:`.
    Shape {
        name: "preview check a person",
        query: "quarterly from:sender3",
        kind: Kind::Executor,
    },
    Shape {
        name: "preview exclude a person",
        query: "quarterly -from:sender3",
        kind: Kind::Executor,
    },
    Shape {
        name: "preview check a label",
        query: "quarterly label:atlas",
        kind: Kind::Executor,
    },
    Shape {
        name: "preview check a folder",
        query: "quarterly in:inbox",
        kind: Kind::Executor,
    },
    Shape {
        name: "timeline range + word",
        query: "quarterly after:2026-07-01 before:2026-10-01",
        kind: Kind::Executor,
    },
    Shape {
        name: "date words since + word",
        query: "quarterly after:2026-07-01",
        kind: Kind::Executor,
    },
    Shape {
        name: "timeline range alone",
        query: "after:2026-07-01 before:2026-10-01",
        kind: Kind::Executor,
    },
    Shape {
        name: "timeline range + operator",
        query: "from:sender3 after:2026-07-01 before:2026-10-01",
        kind: Kind::Executor,
    },
    Shape {
        name: "e2e preview check a person",
        query: "quarterly from:sender3",
        kind: Kind::EndToEnd,
    },
    Shape {
        name: "e2e timeline range + word",
        query: "quarterly after:2026-07-01 before:2026-10-01",
        kind: Kind::EndToEnd,
    },
];

/// Runs the shape once; returns the conversations it found.
async fn run_once(shape: &Shape, query: &ParsedQuery) -> u64 {
    let corpus = corpus();
    let now = today();
    let account = AccountScope::Account(corpus.account.id);
    match shape.kind {
        Kind::Executor => {
            let results = search_conversations(
                &corpus.connection,
                &ConversationRequest {
                    account,
                    query,
                    order: ConversationOrder::BestMatch,
                    offset: 0,
                    limit: 50,
                    today: now.date_naive(),
                },
                now.with_timezone(&Utc),
            )
            .await
            .expect("search");
            std::hint::black_box(&results);
            results.total
        }
        Kind::EndToEnd => {
            let results = postio_session::search::conversations(
                &corpus.connection,
                account,
                query,
                ConversationOrder::BestMatch,
                0,
                50,
            )
            .await
            .expect("conversations");
            let page: Vec<(MessageId, Vec<Source>)> = results
                .hits
                .iter()
                .map(|hit| {
                    (
                        hit.best,
                        hit.matches.iter().map(|m| m.source.clone()).collect(),
                    )
                })
                .collect();
            // Results avoid the first line, which the row shows (D7).
            let passages = postio_session::search::passages(
                &corpus.connection,
                query,
                &page,
                postio_search::passage::FirstLine::Avoided,
            )
            .await;
            std::hint::black_box(&passages);
            results.total
        }
        Kind::Completions => {
            let (prefix, field) = match shape.query.strip_prefix("from:") {
                Some(value) => (value, Some(postio_search::query::Field::From)),
                None => (shape.query, None),
            };
            let found = completions(&corpus.connection, account, prefix, field)
                .await
                .expect("completions");
            std::hint::black_box(&found);
            (found.words.len()
                + found.labels.len()
                + found.lists.len()
                + found.files.len()
                + found.people.len()) as u64
        }
        Kind::Relaxations => {
            let offered = postio_search::relax::relax(query);
            let counts = relaxation_counts(&corpus.connection, account, &offered, now.date_naive())
                .await
                .expect("relaxations");
            counts.iter().filter(|c| **c > 0).count() as u64
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let index = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[index]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    // `cargo bench` also passes `--bench`; nothing here takes arguments.
    postio_model::clock::freeze(today());
    let _ = corpus();
    let today: NaiveDate = today().date_naive();

    println!(
        "{:<28} {:>8} {:>8} {:>8} {:>6} {:>7}",
        "shape", "p50 ms", "p95 ms", "max ms", "stmts", "found"
    );
    let mut over = Vec::new();
    for shape in SHAPES {
        let query = parse(shape.query, today);
        let mut found = 0;
        for _ in 0..WARMUP {
            found = on_runtime(run_once(shape, &query));
        }
        let counts = on_runtime(counting::counted_async(async || {
            on_runtime_inline(shape, &query).await;
        }));
        let mut times: Vec<Duration> = (0..SAMPLES)
            .map(|_| {
                let start = Instant::now();
                found = on_runtime(run_once(shape, &query));
                start.elapsed()
            })
            .collect();
        times.sort();
        let (p50, p95) = (percentile(&times, 0.50), percentile(&times, 0.95));
        println!(
            "{:<28} {:>8.2} {:>8.2} {:>8.2} {:>6} {:>7}",
            shape.name,
            ms(p50),
            ms(p95),
            ms(*times.last().unwrap()),
            counts.statements,
            found
        );
        let budget = match shape.kind {
            Kind::Completions => COMPLETIONS_BUDGET,
            _ => BUDGET,
        };
        if p95 >= budget {
            over.push((shape.name, p95));
        }
    }
    assert!(
        over.is_empty(),
        "over budget at p95 ({BUDGET:?}, completions {COMPLETIONS_BUDGET:?}): {over:?}"
    );
}

async fn on_runtime_inline(shape: &Shape, query: &ParsedQuery) {
    run_once(shape, query).await;
}
