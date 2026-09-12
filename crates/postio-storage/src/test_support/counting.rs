//! What a piece of work cost the database, counted.
//!
//! # Why this is not the instrument it replaces
//!
//! It read SQLite's trace hook: `trace_v2` reported every statement as it
//! finished, with its row count and its VM step count, and `rusqlite/trace`
//! was the binding. This engine has no trace hook, so there is nothing to
//! subscribe to.
//!
//! What is left is this crate's own seam. Every read in the workspace goes
//! through [`crate::sql`] -- `all`, `first`, `one`, `scalar`, `mapped` -- and
//! every one of them is counted here.
//!
//! # What that can and cannot see
//!
//! It sees **statements** and **rows**, which is what the budgets are mostly
//! written in, and it sees them exactly: the counter is incremented where the
//! query is issued, not inferred.
//!
//! It cannot see **steps**, and that is a real loss. #1479 was found by them:
//! `read_receipt_requested_count` was one statement returning one row and
//! walking every message in the store, because an aggregate hides its cost
//! from both other counts. A budget written in statements and rows alone
//! would have passed it.
//!
//! [`scans`] is what replaces that, and it is a different kind of instrument:
//! rather than measuring how much work a query did, it asks the planner
//! whether the query *can* be cheap. An unindexed `count(*)` is a `SCAN`, and
//! `SCAN` on a startup path is the bug #1479 was, caught structurally instead
//! of by a number. It is not a superset -- a query that scans a small table
//! is fine and this flags it -- so a budget using it says which scans it
//! expects.
//!
//! # Availability
//!
//! Behind the `test-support` feature, like the rest of this module. Counting
//! is always on when the feature is compiled in: there is no hook to install,
//! so [`install`] and [`install_on`] are kept only so the suites that call
//! them still read sensibly, and do nothing.

use std::cell::Cell;

/// What one piece of work cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Statements issued through [`crate::sql`].
    ///
    /// The number a budget is usually written in: "opening a window reads a
    /// bounded number of statements however big the mailbox is".
    pub statements: usize,

    /// Rows those statements yielded.
    ///
    /// Rows *returned*, not rows examined -- which is the distinction the
    /// module documentation is about. A `count(*)` over a hundred thousand
    /// messages returns one.
    pub rows: usize,

    /// Kept so the suites that print a `Counts` still compile.
    ///
    /// It was statements issued from inside another statement's callback,
    /// which the trace hook could see and this cannot. Always zero.
    pub nested: usize,

    /// Kept, and always zero. See the module documentation: there is no step
    /// count to read, and [`scans`] is what took over the question.
    pub steps: usize,
}

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
    static ROWS: Cell<usize> = const { Cell::new(0) };
}

/// Count one statement. Called by [`crate::sql`].
pub(crate) fn statement() {
    STATEMENTS.with(|seen| seen.set(seen.get() + 1));
}

/// Count `n` rows. Called by [`crate::sql`].
pub(crate) fn rows(n: usize) {
    ROWS.with(|seen| seen.set(seen.get() + n));
}

/// Does nothing, and is kept so the call sites still read.
///
/// There was a trace hook to install. There is not one now -- counting
/// happens at this crate's own seam and is always on when `test-support` is
/// compiled in.
pub fn install(_connection: &crate::Connection) {}

/// Does nothing. See [`install`].
pub fn install_on(_store: &crate::Store) {}

/// What `body` cost.
///
/// Counts only what happened on *this* thread: the counters are
/// thread-local, so work the body spawned elsewhere is not included. That was
/// true of the trace hook too -- it was per connection, and a spawned task
/// checks out its own.
pub fn counted(body: impl FnOnce()) -> Counts {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    body();
    here()
}

/// What `body` cost, for a body that awaits.
///
/// The async twin of [`counted`], and the one nearly every caller wants now
/// that the storage layer is async.
pub async fn counted_async<F, Fut>(body: F) -> Counts
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    body().await;
    here()
}

/// What has been counted on this thread since the last reset.
pub fn here() -> Counts {
    Counts {
        statements: STATEMENTS.with(Cell::get),
        rows: ROWS.with(Cell::get),
        nested: 0,
        steps: 0,
    }
}

/// Start counting again from zero on this thread.
pub fn reset() {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
}

/// Which steps of `sql`'s plan are full scans.
///
/// The structural half of the instrument, and what took over from the step
/// count. An unindexed aggregate is one statement, one row, and a `SCAN` of
/// the table -- so a budget that cannot see steps can still see *this*, which
/// is the property that actually made #1479 a bug.
///
/// Returns the scanned table names, so a failure says which query is the
/// problem rather than only that there is one.
pub async fn scans(connection: &crate::Connection, sql: &str) -> Vec<String> {
    let steps: Vec<String> = crate::sql::all(
        connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        (),
        |row| crate::sql::RowExt::col(row, 3),
    )
    .await
    .unwrap_or_default();

    steps
        .into_iter()
        .filter(|step| step.starts_with("SCAN"))
        .collect()
}
