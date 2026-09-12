//! A prepared statement, in `rusqlite`'s shapes.

use std::sync::Arc;

use crate::{Connection, Error, Params, Result, Row};

/// A statement prepared against a connection.
///
/// Borrows the connection like `rusqlite`'s does, which is what lets the
/// storage layer's `let mut statement = connection.prepare(..)` compile
/// unchanged.
pub struct Statement<'conn> {
    connection: &'conn Connection,
    inner: turso::Statement,
    names: Arc<Vec<String>>,
}

impl<'conn> Statement<'conn> {
    pub(crate) fn new(connection: &'conn Connection, inner: turso::Statement) -> Self {
        let names = Arc::new(inner.column_names());
        Statement {
            connection,
            inner,
            names,
        }
    }

    /// Run the statement and collect every row.
    ///
    /// **Materialises**, unlike `rusqlite` — see the crate docs. Every paged
    /// read in `postio-storage` is `LIMIT`ed, so this is safe for them; an
    /// unbounded query is the case to watch.
    pub fn query<P: Params>(&mut self, params: P) -> Result<Vec<Row>> {
        let names = Arc::clone(&self.names);
        let values = params.into_values();
        let statement = &mut self.inner;
        self.connection.block_on(async move {
            let mut rows = statement.query(values).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let count = row.column_count();
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    values.push(row.get_value(index)?);
                }
                out.push(Row::new(values, Arc::clone(&names)));
            }
            Ok(out)
        })
    }

    /// Run the statement and map every row through `f`.
    ///
    /// The iterator `rusqlite` returns is lazy and this one is over an
    /// already-collected `Vec`; callers that only `collect()` — which is all
    /// of them in `postio-storage` — cannot tell.
    pub fn query_map<P, T, F>(&mut self, params: P, mut f: F) -> Result<MappedRows<T>>
    where
        P: Params,
        F: FnMut(&Row) -> Result<T>,
    {
        let rows = self.query(params)?;
        let mapped: Vec<Result<T>> = rows.iter().map(&mut f).collect();
        Ok(MappedRows {
            inner: mapped.into_iter(),
        })
    }

    /// The one row the statement returns, mapped by `f`.
    pub fn query_row<P, T, F>(&mut self, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T>,
    {
        let rows = self.query(params)?;
        let row = rows.into_iter().next().ok_or(Error::QueryReturnedNoRows)?;
        f(&row)
    }

    /// Run the statement for its effect, answering how many rows changed.
    pub fn execute<P: Params>(&mut self, params: P) -> Result<usize> {
        let values = params.into_values();
        let statement = &mut self.inner;
        let changed = self
            .connection
            .block_on(async move { statement.execute(values).await })?;
        Ok(changed as usize)
    }

    /// The column names, in order.
    pub fn column_names(&self) -> Vec<String> {
        self.names.as_ref().clone()
    }

    /// How many columns the result has.
    pub fn column_count(&self) -> usize {
        self.inner.column_count()
    }
}

/// What [`Statement::query_map`] hands back: an iterator of results.
pub struct MappedRows<T> {
    inner: std::vec::IntoIter<Result<T>>,
}

impl<T> Iterator for MappedRows<T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
