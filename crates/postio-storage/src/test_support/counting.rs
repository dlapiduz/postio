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

use std::cell::{Cell, RefCell};

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

thread_local! {
    /// One entry per statement currently running, innermost last, `true` if it
    /// is one the code under test asked for.
    ///
    /// A `Row` event names no statement — `StmtRef` keeps its pointer private
    /// — so which statement produced a row has to be inferred from the events
    /// around it. A bare "was the last `Stmt` ours" flag is not enough: a
    /// virtual table runs its own statements *while* the outer one is being
    /// stepped, so an inner lookup between two rows would silently drop the
    /// rest of the outer statement's rows. Statements nest properly and
    /// SQLite reports the end of each one, so a stack is exact where a flag
    /// is approximate.
    static RUNNING: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// Whether `sql` is a statement the code under test issued.
///
/// SQLite reports a nested or internal invocation — a trigger body, a virtual
/// table's own machinery — as an SQL *comment* rather than as statement text.
/// FTS5 is the reason this matters here: one search of a common word showed
/// 1,111 invocations of `SELECT pgno FROM messages_fts_idx ...`, its b-tree
/// segment lookups, which is 2,524 of the 2,584 rows a page of 25 appeared to
/// cost. That number tracks how the index happens to be segmented, not the
/// shape of the query, so counting it would make a budget that fails when
/// SQLite merges segments differently and passes when the application starts
/// reading whole mailboxes.
fn is_application_statement(sql: &str) -> bool {
    !sql.trim_start().starts_with("--")
}

fn record(event: TraceEvent<'_>) {
    match event {
        TraceEvent::Stmt(_, sql) => {
            let ours = is_application_statement(sql);
            RUNNING.with(|running| running.borrow_mut().push(ours));
            if ours {
                STATEMENTS.with(|seen| seen.set(seen.get() + 1));
            }
        }
        TraceEvent::Row(..) => {
            let ours = RUNNING.with(|running| running.borrow().last().copied().unwrap_or(false));
            if ours {
                ROWS.with(|seen| seen.set(seen.get() + 1));
            }
        }
        TraceEvent::Profile(..) => {
            RUNNING.with(|running| running.borrow_mut().pop());
        }
        _ => {}
    }
}

/// Start counting on `connection`. Everything read through it from here on is
/// counted, so install it on the connection the code under test will use.
pub fn install(connection: &Connection) {
    connection.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT
            | TraceEventCodes::SQLITE_TRACE_ROW
            // Not for its timing, which is the thing this module exists to
            // avoid depending on: it is the only event that says a statement
            // finished, which is what keeps `RUNNING` balanced.
            | TraceEventCodes::SQLITE_TRACE_PROFILE,
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
    RUNNING.with(|running| running.borrow_mut().clear());
    body();
    let still_running = RUNNING.with(|running| running.borrow().len());
    assert_eq!(
        still_running, 0,
        "{still_running} statements were still running when the body \
         returned, so rows were attributed against a stack that never \
         unwound. The counts below cannot be trusted."
    );
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
