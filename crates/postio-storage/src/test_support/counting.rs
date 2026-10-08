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
//! There is deliberately **no always-zero `steps` field** standing in for the
//! old one. A budget written as `assert!(counts.steps < BUDGET)` passes
//! trivially against a zero, which is a worse answer than not compiling: the
//! test still runs, still reports green, and no longer asks anything. Every
//! caller that measured steps has had to say what it is really asking.
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
//! so [`install`] is kept only so the suites that call it still read
//! sensibly, and does nothing.

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

    /// Statements compiled rather than taken from the connection's cache.
    ///
    /// Compiling is most of what a small statement costs this engine: a
    /// first sync sampled with `eu-stack` spent more than half its busy time
    /// in `Connection::prepare`, for SQL it had compiled a thousand times
    /// already. A repeated write should compile nothing; this is how a test
    /// says so.
    pub compiles: usize,
}

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
    static ROWS: Cell<usize> = const { Cell::new(0) };
    static COMPILES: Cell<usize> = const { Cell::new(0) };
}

thread_local! {
    /// Every distinct statement text issued on this thread since [`record`],
    /// with how many times it ran. `None` while nothing is recording.
    ///
    /// Thread-local, like the counters beside it, so a test recording its own
    /// work is not handed another test's statements when a harness runs them
    /// as threads of one process. The cost is the counters' own: it sees the
    /// thread the test runs on, so a recording test uses a current-thread
    /// runtime.
    static RECORDED: std::cell::RefCell<Option<std::collections::BTreeMap<String, usize>>> =
        const { std::cell::RefCell::new(None) };
}

/// Start remembering the text of every statement prepared on this thread,
/// discarding any earlier recording. The audit's input: [`recorded`] hands
/// the set back and [`unbounded`] can then ask the planner about each.
pub fn record() {
    RECORDED.with(|seen| *seen.borrow_mut() = Some(Default::default()));
}

/// Stop recording and return each distinct statement with its run count.
pub fn recorded() -> std::collections::BTreeMap<String, usize> {
    RECORDED.with(|seen| seen.borrow_mut().take().unwrap_or_default())
}

/// Note `sql` if recording. Called by [`crate::sql`].
pub(crate) fn note(sql: &str) {
    RECORDED.with(|seen| {
        if let Some(seen) = seen.borrow_mut().as_mut() {
            *seen.entry(sql.to_owned()).or_default() += 1;
        }
    });
}

/// Count one statement. Called by [`crate::sql`].
pub(crate) fn statement() {
    STATEMENTS.with(|seen| seen.set(seen.get() + 1));
}

/// Count `n` rows. Called by [`crate::sql`].
pub(crate) fn rows(n: usize) {
    ROWS.with(|seen| seen.set(seen.get() + n));
}

/// Count one statement compiled. Called by [`crate::sql`].
pub(crate) fn compile() {
    COMPILES.with(|seen| seen.set(seen.get() + 1));
}

/// Note a statement taken through the engine's cache, counting a compile
/// the first time this thread sees its SQL. Called by [`crate::sql`].
///
/// Approximate in one direction: the engine's cache is per connection and
/// this is per thread, so a second connection's first compile goes unseen.
/// What it is for -- a path that compiles on *every* call -- it sees exactly.
pub(crate) fn cached(sql: &str) {
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashSet<String>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
    }
    if SEEN.with(|seen| seen.borrow_mut().insert(sql.to_owned())) {
        compile();
    }
}

/// Does nothing, and is kept so the call sites still read.
///
/// There was a trace hook to install. There is not one now -- counting
/// happens at this crate's own seam and is always on when `test-support` is
/// compiled in.
pub fn install(_connection: &crate::Connection) {}

/// What `body` cost.
///
/// Counts only what happened on *this* thread: the counters are
/// thread-local, so work the body spawned elsewhere is not included. That was
/// true of the trace hook too -- it was per connection, and a spawned task
/// checks out its own.
pub fn counted(body: impl FnOnce()) -> Counts {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    COMPILES.with(|seen| seen.set(0));
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
    COMPILES.with(|seen| seen.set(0));
    body().await;
    here()
}

/// What has been counted on this thread since the last reset.
pub fn here() -> Counts {
    Counts {
        statements: STATEMENTS.with(Cell::get),
        rows: ROWS.with(Cell::get),
        compiles: COMPILES.with(Cell::get),
    }
}

/// Start counting again from zero on this thread.
pub fn reset() {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    COMPILES.with(|seen| seen.set(0));
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

/// The tables whose size follows the mailbox, or the user's history: a read
/// that touches all of one is a cost that grows with the store, and on an
/// encrypted store each page of it is decrypted to be read.
pub const GROWING_TABLES: &[&str] = &[
    "messages",
    "recipients",
    "attachments",
    "search_documents",
    "message_search_bodies",
    "message_headers",
    "threads",
    "thread_links",
    "message_labels",
    "operation_queue",
];

/// The plan of `sql`, one line per step, with every placeholder bound to `1`.
///
/// The planner does not care what the values are, only that there are enough
/// of them; see `test_support::plan`, which this is the non-panicking form of.
pub async fn plan_steps(connection: &crate::Connection, sql: &str) -> crate::Result<Vec<String>> {
    let mut statement =
        crate::sql::statement(connection, &format!("EXPLAIN QUERY PLAN {sql}")).await?;
    let (mut highest, mut bare) = (0, 0);
    let mut rest = sql;
    while let Some(at) = rest.find('?') {
        rest = &rest[at + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            bare += 1;
        }
        highest = highest.max(digits.parse().unwrap_or(0));
        rest = &rest[digits.len()..];
    }
    crate::sql::mapped(&mut statement, vec![1i64; highest.max(bare)], |row| {
        crate::sql::RowExt::col::<String>(row, 3)
    })
    .await
}

/// The steps of `sql`'s plan that read all of a table in `tables`, or all of
/// one scope of it.
///
/// Two shapes, because the second is the first one index level down:
/// - `SCAN t`: every row of the table.
/// - `SEARCH t USING INDEX i (mailbox_id=?)`, or a seek on `account_id` or
///   `target_kind` alone: the index reaches the right folder, account or kind
///   and then every row in it is read and filtered, one table lookup each.
///   That is `needing_backfill_from` walking every body it had already
///   fetched, on each of the hundreds of top-ups a backfill makes.
///
/// A `COVERING` index step is not a walk of the table -- it reads the index
/// and nothing else -- and is not returned. An empty answer is the property:
/// "this statement's cost does not follow the size of the store".
pub async fn unbounded(connection: &crate::Connection, sql: &str, tables: &[&str]) -> Vec<String> {
    let steps = match plan_steps(connection, sql).await {
        Ok(steps) => steps,
        Err(error) => panic!("cannot plan {sql}: {error}"),
    };
    steps
        .into_iter()
        .filter(|step| {
            let on_a_growing_table = step
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| tables.contains(&word));
            on_a_growing_table && (step.starts_with("SCAN") || walks_a_scope(step))
        })
        .collect()
}

/// A seek whose only key is a scope. See [`unbounded`].
fn walks_a_scope(step: &str) -> bool {
    let Some(open) = step.rfind('(') else {
        return false;
    };
    let keys: Vec<&str> = step[open + 1..]
        .trim_end_matches(')')
        .split(" AND ")
        .collect();
    step.starts_with("SEARCH")
        && !step.contains("COVERING INDEX")
        && keys.len() == 1
        && matches!(keys[0], "mailbox_id=?" | "account_id=?" | "target_kind=?")
}

/// How many store connections this process has opened so far.
///
/// Process-wide and monotonic; take a reading before and after, as
/// [`crate::test_support::counting::counted`] does for statements but across
/// threads. A page that costs its own connection costs its own page cache
/// (#1602), so "opening a folder and paging it makes one connection" is a
/// budget in exactly the way "a page is N statements" is.
pub fn checkouts() -> u64 {
    crate::store::CHECKOUTS.load(std::sync::atomic::Ordering::Relaxed)
}
