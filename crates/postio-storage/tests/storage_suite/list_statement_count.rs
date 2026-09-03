//! Listing a page costs the same number of SQL statements at any page size.
//!
//! `docs/PRODUCT.md` §18 states three budgets — 500ms to a usable UI, 16ms per
//! interaction, 100ms for local search — and `CLAUDE.md` calls them "enforced
//! by benches in CI". They are not: `bench.yml` compiles the bench targets and
//! deliberately times nothing, because a shared runner cannot defend 16ms.
//! That decision is right, and it leaves the budgets as documentation (#100).
//!
//! So measure the budget's *cause* rather than its effect. A list stays inside
//! 16ms because of its shape — one page, one statement, over an index — and
//! shape is countable. A count is the same number on a laptop and on a noisy
//! runner, so it can gate a pull request in a way wall-clock never safely can.
//!
//! **What this catches is an N+1.** A per-row lookup added to the list path —
//! a sender, a flag, a thread count fetched message by message — costs
//! milliseconds per row and is invisible to every other test in the workspace,
//! because the rows still come back correct. Here it shows up immediately: the
//! statement count starts scaling with the page.

use std::cell::Cell;

use postio_model::MailboxRole;
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::test_support;

thread_local! {
    /// rusqlite's trace hook is a bare `fn(TraceEvent)` rather than a closure,
    /// so
    /// the counter cannot be captured and has to live where that function can
    /// reach it. Thread-local rather than a global: one case counts at a time,
    /// and a global would make two overlapping counts quietly wrong.
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
}

fn count_one(event: TraceEvent<'_>) {
    // `Stmt` fires as a prepared statement begins running, which is the unit
    // an N+1 multiplies. `Row` would count rows and always differ by design.
    if matches!(event, TraceEvent::Stmt(..)) {
        STATEMENTS.with(|seen| seen.set(seen.get() + 1));
    }
}

/// Statements SQLite prepared while `body` ran.
fn counted(body: impl FnOnce()) -> usize {
    STATEMENTS.with(|seen| seen.set(0));
    body();
    STATEMENTS.with(Cell::get)
}

#[test]
fn listing_a_page_costs_the_same_statements_however_many_rows_it_returns() {
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");
    let connection = database.connection().expect("a connection");

    // Installed on the connection the repository will read through, so the
    // count covers exactly the statements the list path issues.
    connection.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(count_one));

    let page = |limit: u32| ListQuery {
        scope: ListScope::Mailbox(inbox.id),
        limit,
        after: None,
    };
    let messages = MessageRepository::new(&connection);

    // Warm first. The first `prepare` of a statement can pull schema pages in,
    // and this is about the query's shape, not about a cold cache.
    let _ = messages.page(&page(1)).expect("a first read");

    let mut one_row = 0;
    let one = counted(|| one_row = messages.page(&page(1)).expect("one row").len());

    let mut many_rows = 0;
    let many = counted(|| many_rows = messages.page(&page(25)).expect("a page").len());

    assert_eq!(one_row, 1, "a page of one should return one row");
    // Without this the test passes when the hook stops firing: `0 == 0` is a
    // green run that measured nothing. Both reads are one statement today.
    assert!(
        one > 0,
        "the trace hook counted nothing, so the comparison below is vacuous"
    );
    assert!(
        many_rows > 1,
        "a page that returned {many_rows} rows cannot demonstrate an N+1; the \
         seed did not make enough messages for this test to mean anything"
    );
    assert_eq!(
        one, many,
        "listing {many_rows} rows issued {many} statements where one row \
         issued {one}. A list that costs a statement per row is the N+1 that \
         §18's 16ms interaction budget cannot survive — and every other test \
         in this workspace still passes when it is introduced, because the \
         rows come back correct."
    );
}
