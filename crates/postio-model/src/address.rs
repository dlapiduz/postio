//! Mail participants as they appear in address headers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A single RFC 5322 address: an optional display name plus an addr-spec.
///
/// The `address` is stored exactly as it was received. Comparisons that should
/// ignore case go through [`EmailAddress::normalized`] or
/// [`EmailAddress::same_address`]; derived `PartialEq` is exact, because two
/// headers that differ only in display name are genuinely different headers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct EmailAddress {
    /// The display name, if the header carried one.
    pub name: Option<String>,
    /// The addr-spec, e.g. `alice@example.com`, verbatim.
    pub address: String,
}

impl EmailAddress {
    /// Builds an address from an optional display name and an addr-spec.
    pub fn new(name: Option<impl Into<String>>, address: impl Into<String>) -> Self {
        Self {
            name: name.map(Into::into),
            address: address.into(),
        }
    }

    /// The lowercased addr-spec, for indexing and equality.
    pub fn normalized(&self) -> String {
        self.address.to_lowercase()
    }

    /// Whether both values name the same mailbox, ignoring display name and case.
    pub fn same_address(&self, other: &Self) -> bool {
        self.address.eq_ignore_ascii_case(&other.address)
    }

    /// The part before the last `@`.
    pub fn local_part(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(local, _)| local)
    }

    /// The part after the last `@`.
    pub fn domain(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(_, domain)| domain)
    }

    /// What to show in the UI: the display name if there is one, else the address.
    pub fn display(&self) -> &str {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => &self.address,
        }
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => write!(f, "{name} <{}>", self.address),
            _ => f.write_str(&self.address),
        }
    }
}
