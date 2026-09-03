//! What a page of the message list costs, on a mailbox worth worrying about.
//!
//! docs/PRODUCT.md §18 makes two claims about the message list, and this measures
//! both:
//!
//! * **An ordinary interaction is under 16ms.** Scrolling asks for a page,
//!   and a page is a database read — so the read has to fit inside the budget
//!   with room left for the frontend to draw what comes back.
//! * **A mailbox is never loaded into memory.** The stronger claim, and the
//!   one a benchmark can actually settle: reading a page from a mailbox of
//!   100,000 should cost what reading one from a mailbox of 1,000 costs. If
//!   paging were linear in the size of the folder, this is where it shows.
//!
//! # Why the deep page matters
//!
//! `page_at` is an OFFSET query, and OFFSET is not free: SQLite walks the
//! rows it skips. A page at the top of a huge folder proves nothing about a
//! page in the middle of one, so both are measured. If the deep read ever
//! grows away from the shallow one, the answer is a cursor rather than an
//! offset — `MessageRepository::page` already takes one.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-runtime --bench store_reads
//! ```
//!
//! CI compiles this and does not time it: a shared runner is too noisy to
//! trust for a millisecond budget. Each measurement asserts its own budget
//! with a real `Instant` as well, so running it locally fails loudly rather
//! than only reporting.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor now
// reaches bench targets -- the old per-crate `#![warn(missing_docs)]` in
// `lib.rs` never did. A bench is not public API, so documenting a
// macro-generated item would be ceremony rather than information.

use std::hint::black_box;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_model::MailboxRole;
use postio_model::ids::MailboxId;
use postio_runtime::store::{ListScope, MailStore, PageRequest, SqliteStore};
use postio_storage::repository::{ThreadListQuery, ThreadRepository, UnifiedThreadListQuery};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support;

/// A folder big enough that loading it would be the bug.
const HUGE: usize = 100_000;

/// And one small enough to be the control.
const SMALL: usize = 1_000;

/// What the message list asks for at a time.
const PAGE: u32 = 50;

/// A store over a seeded mailbox, and the mailbox to read.
fn seeded(messages: usize) -> (SqliteStore, MailboxId, test_support::TempDatabase) {
    // A file rather than memory: WAL and `mmap_size` are part of what makes
    // the read fast and neither applies to an in-memory database. Measuring
    // the wrong storage engine would be worse than not measuring.
    let database = test_support::temp();
    let report = seed_large(&database, 7, messages);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let store = SqliteStore::new(&database);
    (store, inbox, database)
}

/// Read one page, the way the list does.
fn read(runtime: &tokio::runtime::Runtime, store: &SqliteStore, mailbox: MailboxId, offset: u32) {
    let page = runtime
        .block_on(store.message_page(PageRequest {
            scope: ListScope::Mailbox(mailbox),
            offset,
            limit: PAGE,
        }))
        .expect("the page reads");
    black_box(page);
}

fn bench_message_page(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    let (small, small_inbox, _small_dir) = seeded(SMALL);
    let (huge, huge_inbox, huge_dir) = seeded(HUGE);

    c.bench_function("message page, 1k mailbox", |b| {
        b.iter(|| read(&runtime, &small, small_inbox, 0))
    });
    c.bench_function("message page, 100k mailbox", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, 0))
    });
    // Cold: nobody has read anything between the top and here, so the store
    // has no boundary to seek from and falls back to walking.
    c.bench_function("message page, 100k mailbox, halfway down (cold)", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32))
    });

    // Warm: the page before it has already been read, which is what
    // scrolling does. This is the number that matters, because this is the
    // case that happens — a jump to an unvisited offset happens once, and
    // every page after it is this.
    read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32);
    c.bench_function("message page, 100k mailbox, scrolled to (warm)", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32 + PAGE))
    });

    // Criterion reports; these fail. A budget nobody notices breaking is not
    // a budget, which is why `postio-core`'s own benches assert as well as
    // measure.
    //
    // The two pages a user meets by scrolling: the top of the folder, and one
    // reached from it.
    let deep = (HUGE / 2) as u32;
    read(&runtime, &huge, huge_inbox, deep); // leaves a boundary at deep + PAGE
    for (what, offset) in [("the first page", 0), ("a page scrolled to", deep + PAGE)] {
        read(&runtime, &huge, huge_inbox, offset); // warm the cache
        let start = Instant::now();
        read(&runtime, &huge, huge_inbox, offset);
        let measured = start.elapsed();
        if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
            panic!("{what} of a {HUGE}-message mailbox is over budget: {exceeded:?}");
        }
    }

    // And the jump, which now belongs here too. It was left unasserted
    // because it measured 233ms and could not have met any budget worth
    // setting; #638 found why — the list indexes did not carry the two
    // columns every list query filters on, so the OFFSET skip fetched every
    // row it discarded, and under SQLCipher paid a decrypt for each. With
    // migration 0005 it measures single-digit milliseconds, so a jump is
    // held to the same 16ms as every other thing a person does.
    //
    // A store of its own, because "cold" here means the boundary cache is
    // empty rather than the page cache: `SqliteStore` remembers where each
    // page it has read began, and the reads above have populated it.
    let cold = SqliteStore::new(&huge_dir);
    let start = Instant::now();
    read(&runtime, &cold, huge_inbox, deep);
    let measured = start.elapsed();
    if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
        panic!(
            "a cold jump halfway down a {HUGE}-message mailbox is over \
             budget: {exceeded:?}. The list indexes carry `deleted_locally` \
             and `snoozed_until` so the skip stays inside the index -- see \
             migration 0005 and #638."
        );
    }
}

/// Where the time goes on a big mailbox.
///
/// Kept because it is the explanation, not just a number. Two things were
/// linear in the size of the folder when this was written:
///
/// * `count(*)` — 12ms, and the message list asked for the total with every
///   page. Fixed: a single mailbox answers from `mailboxes.total`, which is
///   maintained against the same predicate the list query uses.
/// * `page_at(offset)` — 25ms halfway down 100,000 messages, because OFFSET
///   walks the rows it skips. Fixed for the case that happens: the store
///   remembers where each page it has read began and seeks from the nearest
///   one, so scrolling costs 176µs at any depth. A jump to a page nobody has
///   visited still walks, once.
fn bench_where_the_time_goes(c: &mut Criterion) {
    let (_store, inbox, database) = seeded(HUGE);
    let connection = database.connection().expect("a connection");
    let messages = postio_storage::repository::MessageRepository::new(&connection);
    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(inbox),
        limit: PAGE,
        after: None,
    };
    c.bench_function("part: count(*)", |b| {
        b.iter(|| black_box(messages.count(&query).expect("count")))
    });
    c.bench_function("part: page_at(0)", |b| {
        b.iter(|| black_box(messages.page_at(&query, 0).expect("page")))
    });
    c.bench_function("part: page_at(50k)", |b| {
        b.iter(|| black_box(messages.page_at(&query, (HUGE / 2) as u32).expect("page")))
    });
}

/// How many messages a seeded conversation holds.
///
/// Four is a real conversation rather than a degenerate one: a thread of one
/// would make the thread list a message list wearing a hat, and the scoped
/// subqueries this measures would each touch a single row.
const PER_THREAD: usize = 4;

/// What a page of the *threaded* list costs (ADR 0015, #306).
///
/// The same two claims as the message page, against the same budget, because
/// a folder that threads is the ordinary folder now — a page of threads is
/// what scrolling the Inbox asks for.
///
/// The shape being measured is the one the ADR chose over view-side
/// collapse: a window over `threads` on `idx_threads_account_last_at`, with
/// the folder's contribution as correlated subqueries **per row of the
/// page**. If those had turned into work proportional to the folder, 100k
/// would separate from 1k here.
fn bench_thread_page(c: &mut Criterion) {
    let seeded_threads = |messages: usize| {
        let database = test_support::temp();
        let report = seed_large(&database, 7, messages);
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
        thread_seeded_messages(&database, report.account.id, PER_THREAD);
        (database, report.account.id, inbox)
    };

    let (small, small_account, small_inbox) = seeded_threads(SMALL);
    let (huge, huge_account, huge_inbox) = seeded_threads(HUGE);

    let read = |database: &test_support::TempDatabase, query: &ThreadListQuery| {
        let connection = database.connection().expect("a connection");
        black_box(
            ThreadRepository::new(&connection)
                .page(query)
                .expect("a page of threads"),
        );
    };

    let small_query = ThreadListQuery::in_mailbox(small_account, small_inbox).limit(PAGE);
    let huge_query = ThreadListQuery::in_mailbox(huge_account, huge_inbox).limit(PAGE);

    c.bench_function("thread page, 1k mailbox", |b| {
        b.iter(|| read(&small, &small_query))
    });
    c.bench_function("thread page, 100k mailbox", |b| {
        b.iter(|| read(&huge, &huge_query))
    });

    // Ten pages in, by cursor — which is how the list actually scrolls, and
    // the case that has to stay flat.
    let deep = {
        let connection = huge.connection().expect("a connection");
        let threads = ThreadRepository::new(&connection);
        let mut query = huge_query.clone();
        for _ in 0..10 {
            let page = threads.page(&query).expect("a page of threads");
            let Some(last) = page.last() else { break };
            query = huge_query.clone().after(last.cursor());
        }
        query
    };
    c.bench_function("thread page, 100k mailbox, ten pages down", |b| {
        b.iter(|| read(&huge, &deep))
    });

    // Criterion reports; this fails. Same reasoning as the message page.
    for (what, query) in [
        ("the first page", &huge_query),
        ("a page scrolled to", &deep),
    ] {
        read(&huge, query);
        let start = Instant::now();
        read(&huge, query);
        let measured = start.elapsed();
        if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
            panic!("{what} of threads over a {HUGE}-message mailbox is over budget: {exceeded:?}");
        }
    }
}

/// What a page of the *unified* list costs (#184, ADR 0005 Q2).
///
/// Two accounts, each with a full mailbox, read as one grouped list. The
/// page walks `idx_threads_last_at` across both and does bounded per-row
/// work — a root lookup and two partner probes per emitted group — so the
/// claim under test is the same as every other page here: the cost is the
/// page's, not the mailbox's, and it fits the interaction budget.
fn bench_unified_page(c: &mut Criterion) {
    let database = test_support::temp();
    let first = seed_large(&database, 7, HUGE / 2);
    thread_seeded_messages(&database, first.account.id, PER_THREAD);
    // A second account of the same size: the unified list's whole point.
    let second = {
        let connection = database.connection().expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .expect("second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX");
        // A modest second corpus, written directly: seed_large seeds one
        // account per store, and what this bench needs from the second is
        // rows to group against, not another 50k of them.
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        connection.execute_batch("BEGIN").expect("begin");
        for i in 0..2_000u32 {
            let mut message = postio_model::Message::new(
                account.id,
                inbox.id,
                chrono::Utc::now() - chrono::Duration::minutes(i as i64),
            );
            message.subject = Some(format!("Cross-account update {i}"));
            messages.create(&mut message).expect("a message");
        }
        connection.execute_batch("COMMIT").expect("commit");
        thread_seeded_messages(&database, account.id, PER_THREAD);
        account.id
    };
    let _ = second;

    let read = |query: &UnifiedThreadListQuery| {
        let connection = database.connection().expect("a connection");
        black_box(
            ThreadRepository::new(&connection)
                .unified_page(query)
                .expect("a unified page"),
        );
    };
    let query = UnifiedThreadListQuery {
        limit: PAGE,
        after: None,
    };

    c.bench_function("unified page, two accounts", |b| b.iter(|| read(&query)));

    // Criterion reports; this fails. Same reasoning as the message page.
    read(&query);
    let start = Instant::now();
    read(&query);
    let measured = start.elapsed();
    if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
        panic!("a unified page over two accounts is over budget: {exceeded:?}");
    }
}

criterion_group!(
    benches,
    bench_message_page,
    bench_thread_page,
    bench_unified_page,
    bench_where_the_time_goes
);
criterion_main!(benches);
