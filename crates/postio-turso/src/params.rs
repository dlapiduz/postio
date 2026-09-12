//! Binding values, in `rusqlite`'s shapes.
//!
//! `postio-storage` binds through three spellings and this crate answers all
//! of them: `[]` for none, `[value]` and `[a, b]` for a slice of one type, and
//! `params![a, b, c]` for a mixed list. There are no `ToSql` implementations
//! in the storage layer itself — checked, not assumed — so the conversions
//! this needs are the primitives and nothing else.

use turso::Value;

/// A value that can be bound to a statement.
pub trait ToSql {
    /// The Turso value it binds as.
    fn to_sql(&self) -> Value;
}

macro_rules! integer {
    ($($ty:ty),*) => {$(
        impl ToSql for $ty {
            fn to_sql(&self) -> Value {
                Value::Integer(*self as i64)
            }
        }
    )*};
}
integer!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl ToSql for f32 {
    fn to_sql(&self) -> Value {
        Value::Real(*self as f64)
    }
}

impl ToSql for f64 {
    fn to_sql(&self) -> Value {
        Value::Real(*self)
    }
}

impl ToSql for bool {
    fn to_sql(&self) -> Value {
        Value::Integer(i64::from(*self))
    }
}

impl ToSql for String {
    fn to_sql(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl ToSql for str {
    fn to_sql(&self) -> Value {
        Value::Text(self.to_owned())
    }
}

impl ToSql for Vec<u8> {
    fn to_sql(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl ToSql for [u8] {
    fn to_sql(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_sql(&self) -> Value {
        match self {
            Some(value) => value.to_sql(),
            None => Value::Null,
        }
    }
}

impl<T: ToSql + ?Sized> ToSql for &T {
    fn to_sql(&self) -> Value {
        (*self).to_sql()
    }
}

/// Everything a call site can pass where `rusqlite` takes parameters.
pub trait Params {
    /// The bound values, in order.
    fn into_values(self) -> Vec<Value>;
}

impl Params for Vec<Value> {
    fn into_values(self) -> Vec<Value> {
        self
    }
}

impl Params for () {
    fn into_values(self) -> Vec<Value> {
        Vec::new()
    }
}

/// The bare `[]` every no-parameter call site writes.
///
/// Its own impl, and a const-generic `[T; N]` cannot coexist with it — which
/// is why `rusqlite` generates the rest by macro and this does too. The
/// comment in its `params.rs` says so in as many words.
impl Params for [&dyn ToSql; 0] {
    fn into_values(self) -> Vec<Value> {
        Vec::new()
    }
}

macro_rules! array_params {
    ($($len:literal),*) => {$(
        impl<T: ToSql> Params for [T; $len] {
            fn into_values(self) -> Vec<Value> {
                self.iter().map(ToSql::to_sql).collect()
            }
        }
    )*};
}
array_params!(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32
);

macro_rules! tuple_params {
    ($(($($name:ident),+)),* $(,)?) => {$(
        #[allow(non_snake_case)]
        impl<$($name: ToSql),+> Params for ($($name,)+) {
            fn into_values(self) -> Vec<Value> {
                let ($($name,)+) = self;
                ::std::vec![$($name.to_sql()),+]
            }
        }
    )*};
}
tuple_params!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H),
);

impl<T: ToSql> Params for &[T] {
    fn into_values(self) -> Vec<Value> {
        self.iter().map(ToSql::to_sql).collect()
    }
}

impl<T: ToSql> Params for Vec<T> {
    fn into_values(self) -> Vec<Value> {
        self.iter().map(ToSql::to_sql).collect()
    }
}

/// `params![a, b, c]`, as `rusqlite` spells it — a mixed list of types.
#[macro_export]
macro_rules! params {
    () => { ::std::vec::Vec::<$crate::__turso::Value>::new() };
    ($($value:expr),+ $(,)?) => {
        ::std::vec![$($crate::ToSql::to_sql(&$value)),+]
    };
}

#[doc(hidden)]
pub mod __turso {
    #[allow(unused_imports)]
    pub use turso::Value;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_spellings_the_storage_layer_uses_all_bind() {
        // `[]`, a slice of one type, and a mixed `params!` list. If one of
        // these stops compiling, a hundred call sites stop with it.
        assert!(().into_values().is_empty());
        assert_eq!([1i64, 2].into_values().len(), 2);
        let mixed = crate::params![1i64, "two", 3.0f64, None::<i64>, true];
        assert_eq!(mixed.len(), 5);
        assert!(matches!(mixed[1], Value::Text(ref t) if t == "two"));
        assert!(matches!(mixed[3], Value::Null));
        assert!(matches!(mixed[4], Value::Integer(1)));
    }
}
