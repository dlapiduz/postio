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
    ///
    /// Undercounts on a full-text path; see [`LAST_WAS_OURS`].
    pub rows: usize,
    /// Invocations SQLite reported as nested: trigger bodies, and the
    /// statements a virtual table runs for itself.
    ///
    /// Per-row work that no other count can see. A trigger fires once per
    /// affected row and produces no result rows, so an index rebuild over a
    /// mailbox shows up here and nowhere else.
    pub nested: usize,
}

thread_local! {
    /// Statements SQLite reported as nested — see [`is_application_statement`].
    static NESTED: Cell<usize> = const { Cell::new(0) };
    /// Whether the last statement to start was one the code under test asked
    /// for. A `Row` event names no statement — `StmtRef` keeps its pointer
    /// private — so a row is attributed to the statement that most recently
    /// began.
    ///
    /// This is exact wherever nothing nests, which is every plain SQL read.
    /// Where something does nest it *under*-counts: an FTS5 cursor runs its
    /// own lookups between two rows of the statement being stepped, and the
    /// rows after one of those are attributed to the lookup instead. That is
    /// the safe direction for a ceiling to be wrong in, and it is why the
    /// search budget is expressed in statements rather than rows.
    ///
    /// A stack would be exact, and cannot be built: it would have to be
    /// unwound by `Profile`, and while SQLite fires `Profile` for the
    /// separately-prepared statements a virtual table runs, it does not fire
    /// one for a trigger body. Trying it left 4,000 statements unclosed over
    /// a 2,000-message index build. Triggers emit no result rows, so nothing
    /// is lost by treating them as an ordinary nested statement here.
    static LAST_WAS_OURS: Cell<bool> = const { Cell::new(false) };
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
            LAST_WAS_OURS.with(|last| last.set(ours));
            if ours {
                STATEMENTS.with(|seen| seen.set(seen.get() + 1));
            } else {
                NESTED.with(|seen| seen.set(seen.get() + 1));
            }
        }
        TraceEvent::Row(..) if LAST_WAS_OURS.with(Cell::get) => {
            ROWS.with(|seen| seen.set(seen.get() + 1));
        }
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
    NESTED.with(|seen| seen.set(0));
    LAST_WAS_OURS.with(|last| last.set(false));
    body();
    let counts = Counts {
        statements: STATEMENTS.with(Cell::get),
        rows: ROWS.with(Cell::get),
        nested: NESTED.with(Cell::get),
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
