use std::hint::black_box;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_model::MailboxRole;
use postio_model::ids::MailboxId;
use postio_runtime::store::{ListScope, LocalStore, PageRequest};
use postio_storage::repository::{ThreadListQuery, ThreadRepository, UnifiedThreadListQuery};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support;

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

const HUGE: usize = 100_000;

const SMALL: usize = 1_000;

const PAGE: u32 = 50;

async fn seeded(messages: usize) -> (LocalStore, MailboxId, test_support::TempStore) {
    // A file rather than memory: WAL and `mmap_size` are part of why the read is fast.
    let database = on_runtime(test_support::temp());
    let report = on_runtime(seed_large(&database, 7, messages));
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let store = LocalStore::new(&database);
    (store, inbox, database)
}

fn read(runtime: &tokio::runtime::Runtime, store: &LocalStore, mailbox: MailboxId, offset: u32) {
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

    let (small, small_inbox, _small_dir) = on_runtime(seeded(SMALL));
    let (huge, huge_inbox, huge_dir) = on_runtime(seeded(HUGE));

    c.bench_function("message page, 1k mailbox", |b| {
        b.iter(|| read(&runtime, &small, small_inbox, 0))
    });
    c.bench_function("message page, 100k mailbox", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, 0))
    });
    // Cold: no boundary to seek from, so the store walks.
    c.bench_function("message page, 100k mailbox, halfway down (cold)", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32))
    });

    // Warm: the page before was read, as scrolling does; this is the case that happens.
    read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32);
    c.bench_function("message page, 100k mailbox, scrolled to (warm)", |b| {
        b.iter(|| read(&runtime, &huge, huge_inbox, (HUGE / 2) as u32 + PAGE))
    });

    // Criterion reports; these fail, so a regression does not go unnoticed.
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

    // The jump was unasserted at 233ms until migration 0005 gave the list indexes both filtered
    // columns (#638). A store of its own, so the boundary cache `LocalStore` keeps is empty.
    let cold = LocalStore::new(&huge_dir);
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

/// Two things were linear in folder size: `count(*)` (now answered from `mailboxes.total`) and
/// `page_at(offset)` (now seeks from the nearest remembered page boundary; a jump to an unvisited
/// page still walks, once).
fn bench_where_the_time_goes(c: &mut Criterion) {
    let (_store, inbox, database) = on_runtime(seeded(HUGE));
    let connection = on_runtime(database.connect()).expect("a connection");
    let messages = postio_storage::repository::MessageRepository::new(&connection);
    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(inbox),
        limit: PAGE,
        after: None,
    };
    c.bench_function("part: count(*)", |b| {
        b.iter(|| black_box(on_runtime(messages.count(&query)).expect("count")))
    });
    c.bench_function("part: page_at(0)", |b| {
        b.iter(|| black_box(on_runtime(messages.page_at(&query, 0)).expect("page")))
    });
    c.bench_function("part: page_at(50k)", |b| {
        b.iter(|| black_box(on_runtime(messages.page_at(&query, (HUGE / 2) as u32)).expect("page")))
    });
}

const PER_THREAD: usize = 4;

/// A window over `threads` on `idx_threads_account_last_at` with correlated subqueries per page
/// row (ADR 0015); 100k must not separate from 1k.
fn bench_thread_page(c: &mut Criterion) {
    let seeded_threads = |messages: usize| {
        let database = on_runtime(test_support::temp());
        let report = on_runtime(seed_large(&database, 7, messages));
        let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
        on_runtime(thread_seeded_messages(
            &database,
            report.account.id,
            PER_THREAD,
        ));
        (database, report.account.id, inbox)
    };

    let (small, small_account, small_inbox) = seeded_threads(SMALL);
    let (huge, huge_account, huge_inbox) = seeded_threads(HUGE);

    let read = |database: &test_support::TempStore, query: &ThreadListQuery| {
        let connection = on_runtime(database.connect()).expect("a connection");
        black_box(
            on_runtime(ThreadRepository::new(&connection).page(query)).expect("a page of threads"),
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

    let deep = {
        let connection = on_runtime(huge.connect()).expect("a connection");
        let threads = ThreadRepository::new(&connection);
        let mut query = huge_query.clone();
        for _ in 0..10 {
            let page = on_runtime(threads.page(&query)).expect("a page of threads");
            let Some(last) = page.last() else { break };
            query = huge_query.clone().after(last.cursor());
        }
        query
    };
    c.bench_function("thread page, 100k mailbox, ten pages down", |b| {
        b.iter(|| read(&huge, &deep))
    });

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

fn bench_unified_page(c: &mut Criterion) {
    let database = on_runtime(test_support::temp());
    let first = on_runtime(seed_large(&database, 7, HUGE / 2));
    on_runtime(thread_seeded_messages(
        &database,
        first.account.id,
        PER_THREAD,
    ));
    let second = {
        let connection = on_runtime(database.connect()).expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        on_runtime(
            postio_storage::repository::AccountRepository::new(&connection).create(&mut account),
        )
        .expect("second account");
        let inbox = on_runtime(test_support::mailbox(&connection, &account, "INBOX"));
        // Written directly: seed_large seeds one account per store and the second only needs rows
        // to group against.
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        on_runtime(connection.execute_batch("BEGIN")).expect("begin");
        for i in 0..2_000u32 {
            let mut message = postio_model::Message::new(
                account.id,
                inbox.id,
                chrono::Utc::now() - chrono::Duration::minutes(i as i64),
            );
            message.subject = Some(format!("Cross-account update {i}"));
            on_runtime(messages.create(&mut message)).expect("a message");
        }
        on_runtime(connection.execute_batch("COMMIT")).expect("commit");
        on_runtime(thread_seeded_messages(&database, account.id, PER_THREAD));
        account.id
    };
    let _ = second;

    let read = |query: &UnifiedThreadListQuery| {
        let connection = on_runtime(database.connect()).expect("a connection");
        black_box(
            on_runtime(ThreadRepository::new(&connection).unified_page(query))
                .expect("a unified page"),
        );
    };
    let query = UnifiedThreadListQuery {
        limit: PAGE,
        after: None,
    };

    c.bench_function("unified page, two accounts", |b| b.iter(|| read(&query)));

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
