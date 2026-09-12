//! Reading a row out, in `rusqlite`'s shapes.

use turso::Value;

use crate::{Error, Result};

/// One row of a result.
pub struct Row {
    values: Vec<Value>,
    names: std::sync::Arc<Vec<String>>,
}

impl Row {
    pub(crate) fn new(values: Vec<Value>, names: std::sync::Arc<Vec<String>>) -> Self {
        Row { values, names }
    }

    /// The column at `index`, as `T`.
    ///
    /// `index` is a position or a column name, which is what lets
    /// `row.get(0)` and `row.get("subject")` both compile — the storage layer
    /// uses both.
    pub fn get<I: RowIndex, T: FromSql>(&self, index: I) -> Result<T> {
        let at = index.index(&self.names)?;
        let value = self
            .values
            .get(at)
            .ok_or_else(|| Error::InvalidColumnName(format!("index {at}")))?;
        T::from_sql(value, at)
    }

    /// The raw value at `index`.
    pub fn get_value<I: RowIndex>(&self, index: I) -> Result<Value> {
        let at = index.index(&self.names)?;
        self.values
            .get(at)
            .cloned()
            .ok_or_else(|| Error::InvalidColumnName(format!("index {at}")))
    }
}

/// A column position, or a column name.
pub trait RowIndex {
    /// Which column this names, in `names`.
    fn index(&self, names: &[String]) -> Result<usize>;
}

impl RowIndex for usize {
    fn index(&self, _names: &[String]) -> Result<usize> {
        Ok(*self)
    }
}

impl RowIndex for i32 {
    fn index(&self, _names: &[String]) -> Result<usize> {
        Ok(*self as usize)
    }
}

impl RowIndex for &str {
    fn index(&self, names: &[String]) -> Result<usize> {
        names
            .iter()
            .position(|name| name == self)
            .ok_or_else(|| Error::InvalidColumnName((*self).to_owned()))
    }
}

/// A type a column can be read as.
pub trait FromSql: Sized {
    /// Read `value`, which came from column `index`.
    fn from_sql(value: &Value, index: usize) -> Result<Self>;
}

fn wrong(index: usize, wanted: &'static str, value: &Value) -> Error {
    Error::InvalidColumnType {
        index,
        wanted,
        found: match value {
            Value::Null => "null".into(),
            Value::Integer(_) => "integer".into(),
            Value::Real(_) => "real".into(),
            Value::Text(_) => "text".into(),
            Value::Blob(_) => "blob".into(),
        },
    }
}

macro_rules! integer {
    ($($ty:ty),*) => {$(
        impl FromSql for $ty {
            fn from_sql(value: &Value, index: usize) -> Result<Self> {
                match value {
                    Value::Integer(n) => Ok(*n as $ty),
                    // SQLite is loosely typed and so is this: a column
                    // declared INTEGER can hold a real, and `rusqlite` reads
                    // it. Refusing here would fail on data SQLite accepted.
                    Value::Real(n) => Ok(*n as $ty),
                    other => Err(wrong(index, stringify!($ty), other)),
                }
            }
        }
    )*};
}
integer!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl FromSql for f64 {
    fn from_sql(value: &Value, index: usize) -> Result<Self> {
        match value {
            Value::Real(n) => Ok(*n),
            Value::Integer(n) => Ok(*n as f64),
            other => Err(wrong(index, "f64", other)),
        }
    }
}

impl FromSql for bool {
    fn from_sql(value: &Value, index: usize) -> Result<Self> {
        match value {
            Value::Integer(n) => Ok(*n != 0),
            other => Err(wrong(index, "bool", other)),
        }
    }
}

impl FromSql for String {
    fn from_sql(value: &Value, index: usize) -> Result<Self> {
        match value {
            Value::Text(text) => Ok(text.clone()),
            // A number read as text is what `CAST` and loose typing produce,
            // and the storage layer meets it in `PRAGMA` results.
            Value::Integer(n) => Ok(n.to_string()),
            Value::Real(n) => Ok(n.to_string()),
            other => Err(wrong(index, "String", other)),
        }
    }
}

impl FromSql for Vec<u8> {
    fn from_sql(value: &Value, index: usize) -> Result<Self> {
        match value {
            Value::Blob(bytes) => Ok(bytes.clone()),
            Value::Text(text) => Ok(text.clone().into_bytes()),
            other => Err(wrong(index, "Vec<u8>", other)),
        }
    }
}

impl<T: FromSql> FromSql for Option<T> {
    fn from_sql(value: &Value, index: usize) -> Result<Self> {
        match value {
            Value::Null => Ok(None),
            other => T::from_sql(other, index).map(Some),
        }
    }
}
