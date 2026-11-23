//! Counts statements, rows and compiles at this crate's own seam (`crate::sql`); the engine has no
//! trace hook. There is no `steps` count: an always-zero field would let a step budget pass
//! trivially. `scans` asks the planner instead (#1479).

use std::cell::Cell;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub statements: usize,

    /// Rows returned, not examined: a `count(*)` over a hundred thousand messages returns one.
    pub rows: usize,

    /// Statements compiled rather than taken from the connection's cache; a repeated write should
    /// compile nothing.
    pub compiles: usize,
}

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
    static ROWS: Cell<usize> = const { Cell::new(0) };
    static COMPILES: Cell<usize> = const { Cell::new(0) };
}

thread_local! {
    /// Distinct statement texts issued on this thread since `record`, with run counts; `None` when
    /// not recording. Thread-local, so a recording test uses a current-thread runtime.
    static RECORDED: std::cell::RefCell<Option<std::collections::BTreeMap<String, usize>>> =
        const { std::cell::RefCell::new(None) };
}

/// Starts remembering the text of every statement prepared on this thread, discarding any earlier
/// recording.
pub fn record() {
    RECORDED.with(|seen| *seen.borrow_mut() = Some(Default::default()));
}

pub fn recorded() -> std::collections::BTreeMap<String, usize> {
    RECORDED.with(|seen| seen.borrow_mut().take().unwrap_or_default())
}

pub(crate) fn note(sql: &str) {
    RECORDED.with(|seen| {
        if let Some(seen) = seen.borrow_mut().as_mut() {
            *seen.entry(sql.to_owned()).or_default() += 1;
        }
    });
}

pub(crate) fn statement() {
    STATEMENTS.with(|seen| seen.set(seen.get() + 1));
}

pub(crate) fn rows(n: usize) {
    ROWS.with(|seen| seen.set(seen.get() + n));
}

pub(crate) fn compile() {
    COMPILES.with(|seen| seen.set(seen.get() + 1));
}

/// Notes a statement taken through the engine's cache, counting a compile the first time this
/// thread sees its SQL. Approximate: the cache is per connection, so a second connection's first
/// compile goes unseen.
pub(crate) fn cached(sql: &str) {
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashSet<String>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
    }
    if SEEN.with(|seen| seen.borrow_mut().insert(sql.to_owned())) {
        compile();
    }
}

/// Does nothing; counting is always on when `test-support` is compiled in.
pub fn install(_connection: &crate::Connection) {}

/// What `body` cost on this thread; work it spawns elsewhere is not counted.
pub fn counted(body: impl FnOnce()) -> Counts {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    COMPILES.with(|seen| seen.set(0));
    body();
    here()
}

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

pub fn here() -> Counts {
    Counts {
        statements: STATEMENTS.with(Cell::get),
        rows: ROWS.with(Cell::get),
        compiles: COMPILES.with(Cell::get),
    }
}

pub fn reset() {
    STATEMENTS.with(|seen| seen.set(0));
    ROWS.with(|seen| seen.set(0));
    COMPILES.with(|seen| seen.set(0));
}

/// Which steps of `sql`'s plan are full scans; returns the scanned table names. Structural, since
/// step counts are gone: an unindexed aggregate is one statement, one row and a `SCAN` (#1479).
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

/// Tables whose size follows the mailbox or the user's history: a read touching all of one grows
/// with the store, and each page is decrypted.
pub const GROWING_TABLES: &[&str] = &[
    "messages",
    "message_contents",
    "message_content_parts",
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

/// The plan of `sql`, one line per step, with every placeholder bound to `1`; the non-panicking
/// `test_support::plan`.
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

/// Steps of `sql`'s plan that read all of a table in `tables` or all of one scope of it: `SCAN t`,
/// or a `SEARCH` keyed only on a scope (`mailbox_id`, `account_id`, `target_kind`) that then
/// filters every row in it. A `COVERING` index step is not a walk and is not returned.
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

/// Connections this process has opened; process-wide and monotonic, so take a reading before and
/// after. A connection costs a page cache (#1602).
pub fn checkouts() -> u64 {
    crate::store::CHECKOUTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Message bodies this process has read and decoded; process-wide and monotonic. A second load is
/// a second decompression that a statement budget sees as one statement among many (#1589).
pub fn body_loads() -> u64 {
    crate::store::BODY_LOADS.load(std::sync::atomic::Ordering::Relaxed)
}
