//! Counting what a query *costs*, in numbers that do not depend on the machine.
//!
//! `docs/PRODUCT.md` §18 states three budgets — 500ms to a usable UI, 16ms per
//! interaction, 100ms for local search — and `CLAUDE.md` calls them "enforced
//! by benches in CI". They are not: `bench.yml` compiles the bench targets and
//! deliberately times nothing, because a shared runner cannot defend 16ms.
//! That decision is right, and it leaves the budgets as documentation (#100).
//!
//! So measure the budget's *cause* rather than its effect. These budgets hold
//! because of the shape of the queries underneath them — one page, one
//! statement, over an index — and shape is countable. A count is the same
//! number on a laptop and on a noisy runner, so it can gate a pull request in
//! a way wall-clock never safely can.
//!
//! Two counts, from SQLite's own trace hook:
//!
//! - **statements** catches an *N+1* — a per-row lookup added to a list path,
//!   which costs milliseconds per row and is invisible to every other test in
//!   the workspace, because the rows still come back correct.
//! - **rows** catches a *full read* — the mailbox pulled into memory and
//!   sliced in Rust. This is the one `page.len()` cannot see: a page that
//!   returns fifty rows returns fifty rows either way, and only the number
//!   SQLite *produced* tells the two apart.

use std::cell::Cell;

use rusqlite::Connection;
use rusqlite::trace::{TraceEvent, TraceEventCodes};

thread_local! {
    /// rusqlite's trace hook is a bare `fn(TraceEvent)` rather than a closure,
    /// so the counters cannot be captured and have to live where that function
    /// can reach them. Thread-local rather than global: one case counts at a
    /// time, and a global would make two overlapping counts quietly wrong.
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
    static ROWS: Cell<usize> = const { Cell::new(0) };
}

/// What SQLite did while a body ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Statements that began running — the unit an N+1 multiplies.
    pub statements: usize,
    /// Rows those statements produced, whether or not the caller kept them.
    pub rows: usize,
}

fn record(event: TraceEvent<'_>) {
    match event {
        TraceEvent::Stmt(..) => STATEMENTS.with(|seen| seen.set(seen.get() + 1)),
        TraceEvent::Row(..) => ROWS.with(|seen| seen.set(seen.get() + 1)),
        _ => {}
    }
}

/// Start counting on `connection`. Everything read through it from here on is
/// counted, so install it on the connection the code under test will use.
pub fn install(connection: &Connection) {
    connection.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT | TraceEventCodes::SQLITE_TRACE_ROW,
        Some(record),
    );
}

/// What SQLite did while `body` ran, on any connection [`install`] was called
/// on. Panics rather than returning zero: a count of nothing is not a cheap
/// query, it is a measurement that did not happen, and `0 <= budget` is a
/// green run that proves nothing at all.
pub fn counted(body: impl FnOnce()) -> Counts {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    body();
    let counts = Counts {
        statements: STATEMENTS.with(Cell::get),
        rows: ROWS.with(Cell::get),
    };
    assert!(
        counts.statements > 0,
        "the trace hook counted no statements at all, so any budget compared \
         against these counts would pass without measuring anything. Either \
         `install` was not called on the connection the body reads through, \
         or the body issued no query."
    );
    counts
}
