//! Raw header storage.

use serde::{Deserialize, Serialize};

/// One header field, name and unfolded value, as it appeared on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// The field name, original case preserved.
    pub name: String,
    /// The unfolded field value.
    pub value: String,
}

impl Header {
    /// Builds a header field.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A message's headers.
///
/// Order and duplicates are preserved — `Received` chains and multiple
/// `References` lines matter — while lookup is case-insensitive per RFC 5322.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Headers(Vec<Header>);

impl Headers {
    /// An empty header block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a header field, keeping any existing field of the same name.
    pub fn push(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push(Header::new(name, value));
    }

    /// The first value for `name`, matched case-insensitively.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    /// Every value for `name`, in wire order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
            .collect()
    }

    /// Whether a field with this name is present.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Iterates the fields in wire order.
    pub fn iter(&self) -> std::slice::Iter<'_, Header> {
        self.0.iter()
    }

    /// The number of header fields, counting duplicates.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no header fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Header> for Headers {
    fn from_iter<I: IntoIterator<Item = Header>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<N: Into<String>, V: Into<String>> FromIterator<(N, V)> for Headers {
    fn from_iter<I: IntoIterator<Item = (N, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(name, value)| Header::new(name, value))
                .collect(),
        )
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = &'a Header;
    type IntoIter = std::slice::Iter<'a, Header>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
