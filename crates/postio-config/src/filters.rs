//! `[filters]` — named saved queries.
//!
//! ```toml
//! [filters.needs-reply]
//! query = "is:unread from:team"
//! pinned = true
//! ```
//!
//! The query string uses the same operator language as the search bar
//! (`from:`, `has:attach`, `is:unread`, …); parsing it belongs to
//! `postio-search`, so this layer keeps it as text.

use serde::{Deserialize, Serialize};

use crate::Extras;

/// One entry of `[filters]`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    /// The search expression.
    #[serde(default)]
    pub query: String,
    /// Show this filter in the sidebar.
    #[serde(default)]
    pub pinned: bool,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}
