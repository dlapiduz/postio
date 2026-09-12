//! Reading rows, and the three query shapes this crate actually uses.
//!
//! # Why this exists
//!
//! The engine's own row accessor is `Row::get::<T>`, over a `FromValue` trait
//! that is **sealed** and has no `Option<T>`: a NULL is an error rather than a
//! `None`. Most columns in this schema are nullable, so almost every read
//! would be a `match` on `get_value`. [`RowExt`] is that `match`, written once.
//!
//! It is not a port of anyone's API. It is eight accessors and three
//! functions, shaped by what the repositories in this crate ask for, and it
//! returns [`crate::Error`] rather than inventing an error type of its own.
//!
//! # The rule the query helpers exist to keep
//!
//! **A `Rows` holds its connection until it is dropped.** The next statement
//! on the same connection fails at runtime with "connection is busy with
//! another operation" — there is no borrow to catch it at compile time, the
//! way `rusqlite`'s `Statement` borrowing its `Connection` did. So [`all`] and
//! [`first`] collect and drop before returning, and nothing in this crate
//! holds a `Rows` across a write.

use turso::{Connection, IntoParams, Row, Value};

use crate::error::{Error, Result};

/// Typed column access, with NULL as `None` rather than as an error.
pub(crate) trait RowExt {
    /// The raw value, for a caller that wants to branch on its type.
    fn value(&self, index: usize) -> Result<Value>;

    /// A TEXT column that the schema says is `NOT NULL`.
    fn text(&self, index: usize) -> Result<String>;
    /// A TEXT column that may be NULL.
    fn opt_text(&self, index: usize) -> Result<Option<String>>;

    /// An INTEGER column that the schema says is `NOT NULL`.
    fn int(&self, index: usize) -> Result<i64>;
    /// An INTEGER column that may be NULL.
    fn opt_int(&self, index: usize) -> Result<Option<i64>>;

    /// An INTEGER column read as a flag. Anything non-zero is true, which is
    /// what `count(*) > 0` and `EXISTS` produce.
    fn flag(&self, index: usize) -> Result<bool>;

    /// A REAL column.
    fn real(&self, index: usize) -> Result<f64>;

    /// A BLOB column that may be NULL.
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
            // An aggregate over no rows, and `sum()` in particular, comes back
            // REAL often enough to be worth accepting here rather than at
            // every call site.
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

/// Every row the query returns, mapped.
///
/// Collects before returning, so the `Rows` is dropped and the connection is
/// free for whatever the caller does next. That is not an optimisation to
/// undo: see the module documentation.
pub(crate) async fn all<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    mut map: F,
) -> Result<Vec<T>>
where
    F: FnMut(&Row) -> Result<T>,
{
    let mut rows = connection.query(sql, params).await?;
    let mut mapped = Vec::new();
    while let Some(row) = rows.next().await? {
        mapped.push(map(&row)?);
    }
    drop(rows);
    Ok(mapped)
}

/// The first row, mapped, or `None` if there were none.
///
/// A missing row is not an error — the repositories' `get` convention — so
/// this returns `Option` rather than failing.
pub(crate) async fn first<T, F>(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: F,
) -> Result<Option<T>>
where
    F: FnOnce(&Row) -> Result<T>,
{
    let mut rows = connection.query(sql, params).await?;
    let mapped = match rows.next().await? {
        Some(row) => Some(map(&row)?),
        None => None,
    };
    drop(rows);
    Ok(mapped)
}

/// A single-column aggregate: `count(*)`, `max(id)`, `sum(size)`.
///
/// Returns `0` when the query produced no row at all, which `count(*)` never
/// does and `max()` over an empty table does.
pub(crate) async fn scalar(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
) -> Result<i64> {
    Ok(first(connection, sql, params, |row| row.opt_int(0))
        .await?
        .flatten()
        .unwrap_or(0))
}

/// Whether the query matched anything.
pub(crate) async fn exists(
    connection: &Connection,
    sql: &str,
    params: impl IntoParams,
) -> Result<bool> {
    Ok(first(connection, sql, params, |_| Ok(())).await?.is_some())
}

/// One name for every scope. The engine resolves `RELEASE`/`ROLLBACK TO`
/// against the most recent savepoint of that name, and scopes nest lexically,
/// so they are always released in the order that resolution expects.
const SAVEPOINT: &str = "postio_scope";

/// Run `work` inside an atomic scope, committing if it succeeds and rolling
/// back if it does not.
///
/// # Why a closure, and not a guard with a `Drop`
///
/// It was a guard: `Scope` held the connection, and its destructor rolled back
/// unless `commit` had been called, so an early `?` could not leave half a
/// write behind. A destructor cannot `await`, and rolling back is now a
/// statement like any other — so the scope is a closure instead, and the early
/// `?` becomes the closure returning `Err`, which is the same guarantee
/// arrived at by the one route async leaves open.
///
/// The connection is handed to `work` by clone. That is not a second
/// connection: `Connection` is a handle onto a shared inner, so the clone is
/// inside the very transaction this opened.
///
/// # The outermost scope is `BEGIN IMMEDIATE`, and that is load-bearing
///
/// A bare `SAVEPOINT` outside any transaction *starts* one, and the one it
/// starts is deferred — it takes no lock until something asks for one. Every
/// scope here reads before it writes, because that is what a read-modify-write
/// is. So a deferred scope holds a *read* lock by the time it writes and has
/// to promote, and a promotion cannot wait: blocking a connection that already
/// holds a read lock could deadlock against the writer it would be waiting
/// for, so the engine refuses on the spot rather than running the busy
/// handler.
///
/// Postio always has a second writer — the UI thread, writing local-first on
/// every flag, archive and draft autosave — so this is not theoretical: #79
/// found a sync pass losing its first batch to an `f` keystroke. Taking the
/// write lock up front is what puts these writes back inside the timeout.
///
/// A nested scope stays a plain `SAVEPOINT`: the transaction enclosing it has
/// already answered the question, and asking again would be a second `BEGIN`.
pub(crate) async fn in_scope<T, F, Fut>(connection: &Connection, work: F) -> Result<T>
where
    F: FnOnce(Connection) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    // `is_autocommit` is false exactly when a transaction is already open,
    // which is what "am I nested" means here — whether the enclosing
    // transaction came from another scope or from a caller's own `BEGIN`
    // makes no difference to what this one has to do.
    let outermost = connection.is_autocommit()?;

    connection
        .execute(
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
            connection
                .execute(
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
            // Best effort: the caller is already carrying an error, and a
            // failure to roll back surfaces on the next statement.
            if outermost {
                let _ = connection.execute("ROLLBACK", ()).await;
            } else {
                let _ = connection
                    .execute(&format!("ROLLBACK TO {SAVEPOINT}"), ())
                    .await;
                let _ = connection
                    .execute(&format!("RELEASE {SAVEPOINT}"), ())
                    .await;
            }
            Err(error)
        }
    }
}
