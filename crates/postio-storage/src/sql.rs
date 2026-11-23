//! Typed row reads for the engine, whose `Row::get` has no `Option<T>`: a NULL is an error.
//! A `Rows` holds its connection until dropped and the next statement fails "connection is busy",
//! so `all` and `first` collect and drop; never hold one across a write.

use turso::{Connection, IntoParams, Row, Value};

use crate::error::{Error, Result};

pub trait FromColumn: Sized {
    fn from_column(value: Value, index: usize) -> Result<Self>;
}

impl FromColumn for Value {
    fn from_column(value: Value, _index: usize) -> Result<Self> {
        Ok(value)
    }
}

impl FromColumn for String {
    fn from_column(value: Value, index: usize) -> Result<Self> {
        match value {
            Value::Text(text) => Ok(text),
            other => Err(wrong_type(index, "TEXT", &other)),
        }
    }
}

impl FromColumn for Vec<u8> {
    fn from_column(value: Value, index: usize) -> Result<Self> {
        match value {
            Value::Blob(bytes) => Ok(bytes),
            Value::Text(text) => Ok(text.into_bytes()),
            other => Err(wrong_type(index, "BLOB", &other)),
        }
    }
}

impl FromColumn for bool {
    fn from_column(value: Value, index: usize) -> Result<Self> {
        Ok(i64::from_column(value, index)? != 0)
    }
}

impl FromColumn for f64 {
    fn from_column(value: Value, index: usize) -> Result<Self> {
        match value {
            Value::Real(number) => Ok(number),
            Value::Integer(number) => Ok(number as f64),
            other => Err(wrong_type(index, "REAL", &other)),
        }
    }
}

impl<T: FromColumn> FromColumn for Option<T> {
    fn from_column(value: Value, index: usize) -> Result<Self> {
        match value {
            Value::Null => Ok(None),
            other => T::from_column(other, index).map(Some),
        }
    }
}

/// A narrowing that does not fit is an error, not a wrap: truncating a UID or port is worse.
macro_rules! integer_column {
    ($($ty:ty),* $(,)?) => {$(
        impl FromColumn for $ty {
            fn from_column(value: Value, index: usize) -> Result<Self> {
                let number = match value {
                    Value::Integer(number) => number,
                    // `sum()`, `avg()` and integer arithmetic come back REAL.
                    Value::Real(number) => number as i64,
                    other => return Err(wrong_type(index, "INTEGER", &other)),
                };
                <$ty>::try_from(number).map_err(|_| Error::ColumnType {
                    column: format!("column {index}"),
                    reason: format!(
                        "{number} does not fit in {}",
                        std::any::type_name::<$ty>()
                    ),
                })
            }
        }
    )*};
}

integer_column!(i64, i32, i16, i8, u64, u32, u16, u8, usize, isize);

pub trait RowExt {
    fn col<T: FromColumn>(&self, index: usize) -> Result<T>;

    fn value(&self, index: usize) -> Result<Value>;

    fn text(&self, index: usize) -> Result<String>;
    fn opt_text(&self, index: usize) -> Result<Option<String>>;

    fn int(&self, index: usize) -> Result<i64>;
    fn opt_int(&self, index: usize) -> Result<Option<i64>>;

    /// Non-zero is true, as `count(*) > 0` and `EXISTS` produce.
    fn flag(&self, index: usize) -> Result<bool>;

    fn real(&self, index: usize) -> Result<f64>;

    fn opt_blob(&self, index: usize) -> Result<Option<Vec<u8>>>;
}

fn wrong_type(index: usize, wanted: &str, got: &Value) -> Error {
    Error::ColumnType {
        column: format!("column {index}"),
        reason: format!("expected {wanted}, found {}", describe(got)),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "NULL",
        Value::Integer(_) => "INTEGER",
        Value::Real(_) => "REAL",
        Value::Text(_) => "TEXT",
        Value::Blob(_) => "BLOB",
    }
}

impl RowExt for Row {
    fn col<T: FromColumn>(&self, index: usize) -> Result<T> {
        T::from_column(self.value(index)?, index)
    }

    fn value(&self, index: usize) -> Result<Value> {
        self.get_value(index).map_err(Into::into)
    }

    fn text(&self, index: usize) -> Result<String> {
        match self.value(index)? {
            Value::Text(text) => Ok(text),
            other => Err(wrong_type(index, "TEXT", &other)),
        }
    }

    fn opt_text(&self, index: usize) -> Result<Option<String>> {
        match self.value(index)? {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => Err(wrong_type(index, "TEXT or NULL", &other)),
        }
    }

    fn int(&self, index: usize) -> Result<i64> {
        match self.value(index)? {
            Value::Integer(number) => Ok(number),
            // An aggregate over no rows, `sum()` in particular, comes back REAL.
            Value::Real(number) => Ok(number as i64),
            other => Err(wrong_type(index, "INTEGER", &other)),
        }
    }

    fn opt_int(&self, index: usize) -> Result<Option<i64>> {
        match self.value(index)? {
            Value::Null => Ok(None),
            Value::Integer(number) => Ok(Some(number)),
            Value::Real(number) => Ok(Some(number as i64)),
            other => Err(wrong_type(index, "INTEGER or NULL", &other)),
        }
    }

    fn flag(&self, index: usize) -> Result<bool> {
        Ok(self.opt_int(index)?.unwrap_or(0) != 0)
    }

    fn real(&self, index: usize) -> Result<f64> {
        match self.value(index)? {
            Value::Real(number) => Ok(number),
            Value::Integer(number) => Ok(number as f64),
            other => Err(wrong_type(index, "REAL", &other)),
        }
    }

    fn opt_blob(&self, index: usize) -> Result<Option<Vec<u8>>> {
        match self.value(index)? {
            Value::Null => Ok(None),
            Value::Blob(bytes) => Ok(Some(bytes)),
            other => Err(wrong_type(index, "BLOB or NULL", &other)),
        }
    }
}

#[inline]
fn count(rows: usize) {
    #[cfg(feature = "test-support")]
    {
        crate::test_support::counting::statement();
        crate::test_support::counting::rows(rows);
    }
    #[cfg(not(feature = "test-support"))]
    let _ = rows;
}

#[inline]
fn count_compile() {
    #[cfg(feature = "test-support")]
    crate::test_support::counting::compile();
}

/// Above this many placeholders a statement is compiled every time: the engine's statement cache
/// never evicts, so only a finite set of spellings may enter it, and `IN (?, ?, ?)` lists vary
/// without bound.
const CACHED_PLACEHOLDERS: usize = 64;

/// A statement for `sql`, compiled once per connection. Every statement here is prepared through
/// this (clippy.toml refuses `Connection::execute`, `query` and `prepare`) because recompiling is
/// most of a small statement's cost. A cached statement does not finish a dropped, unfinished
/// `turso::Transaction`; use `in_scope`.
pub async fn statement(connection: &Connection, sql: &str) -> Result<turso::Statement> {
    #[cfg(feature = "test-support")]
    crate::test_support::counting::note(sql);
    if placeholders_in(sql) > CACHED_PLACEHOLDERS {
        count_compile();
        #[allow(clippy::disallowed_methods)]
        return Ok(connection.prepare(sql).await?);
    }
    #[cfg(feature = "test-support")]
    crate::test_support::counting::cached(sql);
    Ok(connection.prepare_cached(sql).await?)
}

pub async fn execute(connection: &Connection, sql: &str, params: impl IntoParams) -> Result<u64> {
    Ok(statement(connection, sql).await?.execute(params).await?)
}

fn placeholders_in(sql: &str) -> usize {
    let mut count = 0;
    let mut quote = None;
    for byte in sql.bytes() {
        match (quote, byte) {
            (Some(open), _) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'?') => count += 1,
            _ => {}
        }
    }
    count
}

/// Runs a batch of `;`-separated statements and counts them; `execute_batch` is uncounted, which
/// hides schema cost from a startup budget (#1113). The count scans for `;` outside string
/// literals and `--` comments, so it is approximate.
pub async fn batch(connection: &Connection, sql: &str) -> Result<()> {
    connection.execute_batch(sql).await?;
    count_statements(statements_in(sql));
    Ok(())
}

fn statements_in(sql: &str) -> usize {
    let mut found = 0;
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b';' => found += 1,
            _ => {}
        }
        i += 1;
    }
    found
}

#[inline]
fn count_statements(n: usize) {
    #[cfg(feature = "test-support")]
    for _ in 0..n {
        crate::test_support::counting::statement();
    }
    #[cfg(not(feature = "test-support"))]
    let _ = n;
}

/// Rows one read may return before it counts as unbounded. Debug builds only: a development guard,
/// not a runtime limit. A query with its own `LIMIT` is exempt; a read that wants every row calls
/// `all_unbounded`.
pub const UNBOUNDED_READ: usize = 3_000;

/// Collects before returning, so the connection is free for the next statement. Panics in debug
/// builds if the query returns more than `UNBOUNDED_READ` rows.
pub async fn all<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&Row) -> Result<T>,
{
    let mut rows = statement(connection, sql).await?.query(params).await?;
    let mut mapped = Vec::new();
    while let Some(row) = rows.next().await? {
        mapped.push(map(&row)?);
    }
    drop(rows);
    count(mapped.len());
    debug_assert!(
        mapped.len() <= UNBOUNDED_READ || states_a_limit(sql),
        "an unbounded read: {} rows from a single query, over the \
         {UNBOUNDED_READ}-row guard. §18's claim is that a whole mailbox is \
         never loaded into memory, and this is what that looks like when it \
         stops being true. Add a LIMIT, or call `all_unbounded` if every row \
         is genuinely what the caller needs:\n  {sql}",
        mapped.len(),
    );
    Ok(mapped)
}

/// Textual: looks for `LIMIT` outside string literals, since `LIKE '%limit%'` is not a bound.
fn states_a_limit(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
            }
            b'l' | b'L' if sql[i..].len() >= 5 && sql[i..i + 5].eq_ignore_ascii_case("limit") => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// `all` for a read that genuinely wants every row, exempt from `UNBOUNDED_READ`.
pub async fn all_unbounded<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&Row) -> Result<T>,
{
    let mut rows = statement(connection, sql).await?.query(params).await?;
    let mut mapped = Vec::new();
    while let Some(row) = rows.next().await? {
        mapped.push(map(&row)?);
    }
    drop(rows);
    count(mapped.len());
    Ok(mapped)
}

/// Hands each row to `visit` until it answers `false`; returns how many rows were visited. Stopping
/// drops the rows, so the connection is free for the next statement.
pub async fn each<F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    mut visit: F,
) -> Result<usize>
where
    F: FnMut(&Row) -> Result<bool>,
{
    let mut rows = statement(connection, sql).await?.query(params).await?;
    let mut visited = 0;
    while let Some(row) = rows.next().await? {
        visited += 1;
        if !visit(&row)? {
            break;
        }
    }
    drop(rows);
    count(visited);
    Ok(visited)
}

/// `None` if there were no rows.
pub async fn first<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: F,
) -> Result<Option<T>>
where
    F: FnOnce(&Row) -> Result<T>,
{
    let mut rows = statement(connection, sql).await?.query(params).await?;
    let mapped = match rows.next().await? {
        Some(row) => Some(map(&row)?),
        None => None,
    };
    drop(rows);
    count(usize::from(mapped.is_some()));
    Ok(mapped)
}

/// `all` over an already-prepared statement, for SQL built at runtime.
pub async fn mapped<T, F>(
    statement: &mut turso::Statement,
    params: impl IntoParams,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&Row) -> Result<T>,
{
    let mut rows = statement.query(params).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(map(&row)?);
    }
    drop(rows);
    count(out.len());
    Ok(out)
}

/// `first` over an already-prepared statement. Counted, which a raw `statement.query(..).next()`
/// is not.
pub async fn first_of<T, F>(
    statement: &mut turso::Statement,
    params: impl IntoParams,
    map: F,
) -> Result<Option<T>>
where
    F: FnOnce(&Row) -> Result<T>,
{
    let mut rows = statement.query(params).await?;
    let mapped = match rows.next().await? {
        Some(row) => Some(map(&row)?),
        None => None,
    };
    drop(rows);
    count(usize::from(mapped.is_some()));
    Ok(mapped)
}

/// The first row, where there must be one: `Error::NotFound` rather than a panic.
pub async fn one<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: F,
) -> Result<T>
where
    F: FnOnce(&Row) -> Result<T>,
{
    first(connection, sql, params, map)
        .await?
        .ok_or(Error::NotFound {
            entity: "row",
            id: 0,
        })
}

/// A single-column aggregate; `0` when the query produced no row (`max()` over an empty table).
pub async fn scalar(connection: &Connection, sql: &str, params: impl IntoParams) -> Result<i64> {
    Ok(first(connection, sql, params, |row| row.opt_int(0))
        .await?
        .flatten()
        .unwrap_or(0))
}

pub async fn exists(connection: &Connection, sql: &str, params: impl IntoParams) -> Result<bool> {
    Ok(first(connection, sql, params, |_| Ok(())).await?.is_some())
}

/// One name for every scope: the engine resolves `RELEASE`/`ROLLBACK TO` against the most recent
/// savepoint of that name, and scopes nest lexically.
const SAVEPOINT: &str = "postio_scope";

/// Runs `work` in an atomic scope, committing on `Ok` and rolling back on `Err`. A closure, not a
/// `Drop` guard, because rollback is a statement and a destructor cannot await.
/// The outermost scope is `BEGIN IMMEDIATE`: a deferred one holds a read lock when it writes and
/// cannot promote while the UI thread also writes, so the engine refuses at once (#79).
pub async fn in_scope<T, E, F, Fut>(connection: &Connection, work: F) -> std::result::Result<T, E>
where
    E: From<Error>,
    F: FnOnce(Connection) -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, E>>,
{
    // `is_autocommit` is false exactly when a transaction is open, whoever opened it.
    let outermost = connection.is_autocommit().map_err(Error::from)?;

    execute(
        connection,
        if outermost {
            "BEGIN IMMEDIATE"
        } else {
            "SAVEPOINT postio_scope"
        },
        (),
    )
    .await?;

    match work(connection.clone()).await {
        Ok(value) => {
            execute(
                connection,
                if outermost {
                    "COMMIT"
                } else {
                    "RELEASE postio_scope"
                },
                (),
            )
            .await?;
            Ok(value)
        }
        Err(error) => {
            // Best effort: the caller already carries an error.
            if outermost {
                let _ = execute(connection, "ROLLBACK", ()).await;
            } else {
                let _ = execute(connection, &format!("ROLLBACK TO {SAVEPOINT}"), ()).await;
                let _ = execute(connection, &format!("RELEASE {SAVEPOINT}"), ()).await;
            }
            Err(error)
        }
    }
}

/// What a value binds to, by borrow: `turso::params!` moves, so `params![account.display_name]`
/// against a `&Account` does not compile.
pub trait Bind {
    fn bind(&self) -> Value;
}

impl Bind for String {
    fn bind(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl Bind for str {
    fn bind(&self) -> Value {
        Value::Text(self.to_owned())
    }
}

impl Bind for bool {
    fn bind(&self) -> Value {
        Value::Integer(i64::from(*self))
    }
}

impl Bind for f64 {
    fn bind(&self) -> Value {
        Value::Real(*self)
    }
}

impl Bind for Vec<u8> {
    fn bind(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl Bind for [u8] {
    fn bind(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}

impl Bind for Value {
    fn bind(&self) -> Value {
        self.clone()
    }
}

impl<T: Bind + ?Sized> Bind for &T {
    fn bind(&self) -> Value {
        (**self).bind()
    }
}

impl<T: Bind> Bind for Option<T> {
    fn bind(&self) -> Value {
        match self {
            Some(value) => value.bind(),
            None => Value::Null,
        }
    }
}

/// Widens to `i64`, the only integer the engine stores. A `u64` too large is clamped: the only one
/// here is a `ModSeq`, and failing a sync write over it is worse.
macro_rules! bind_integer {
    ($($ty:ty),* $(,)?) => {$(
        impl Bind for $ty {
            fn bind(&self) -> Value {
                Value::Integer(i64::try_from(*self).unwrap_or(i64::MAX))
            }
        }
    )*};
}

bind_integer!(i64, i32, i16, i8, u64, u32, u16, u8, usize, isize);

#[macro_export]
macro_rules! bind {
    () => { () };
    ($($value:expr),* $(,)?) => {
        [$($crate::sql::Bind::bind(&$value)),*]
    };
}

pub use crate::bind;
