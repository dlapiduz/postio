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
use postio_storage::seed::seed_large;
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
    let (huge, huge_inbox, _huge_dir) = seeded(HUGE);

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
    // Criterion reports; this fails. A budget nobody notices breaking is not
    // a budget, which is why `postio-core`'s own benches assert as well as
    // measure.
    //
    // Both the pages a user actually meets: the top of the folder, and one
    // reached by scrolling. A *jump* to a page nobody has visited still walks
    // — that is the `OFFSET` fallback, it happens once per jump, and every
    // page after it is the warm case measured here.
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

criterion_group!(benches, bench_message_page, bench_where_the_time_goes);
criterion_main!(benches);
