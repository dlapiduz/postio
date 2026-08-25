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

use crate::{Config, Extras};

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

impl Config {
    /// Save `query` as a new pinned filter, named from the query text.
    ///
    /// This is `Ctrl+S`'s write path (issue #10): the caller still has to
    /// call [`Config::save_to_path`] afterward, the same as any other
    /// programmatic edit -- this only touches the in-memory table. Returns
    /// the key the filter was stored under, since the name is derived rather
    /// than chosen and a caller updating a sidebar needs to know it.
    pub fn save_filter(&mut self, query: &str) -> String {
        let key = self.unique_filter_key(query);
        self.filters.insert(
            key.clone(),
            FilterConfig {
                query: query.to_string(),
                pinned: true,
                extra: Extras::default(),
            },
        );
        key
    }

    /// A `[filters]` key derived from `query`, distinct from every key
    /// already in the table.
    ///
    /// Two searches saved with the same text is not a conflict a user should
    /// have to resolve by typing a name first -- it is `-2`, `-3`, the way a
    /// file manager handles a second "Untitled".
    fn unique_filter_key(&self, query: &str) -> String {
        let base = slug(query);
        if !self.filters.contains_key(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|candidate| !self.filters.contains_key(candidate))
            .expect("an unbounded counter always finds a free key")
    }
}

/// Lowercase, hyphen-separated, and never empty -- a `[filters.<key>]` table
/// name has to be a bare TOML key, so anything that is not ASCII
/// alphanumeric becomes one separator rather than surviving into it.
fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = true; // No leading hyphen.
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    let slug = slug.strip_suffix('-').map(str::to_string).unwrap_or(slug);
    if slug.is_empty() {
        "search".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_a_query_names_it_from_the_query_and_pins_it() {
        let mut config = Config::default();
        let key = config.save_filter("is:unread from:team");
        assert_eq!(key, "is-unread-from-team");
        let saved = config.filters.get(&key).expect("the new filter");
        assert_eq!(saved.query, "is:unread from:team");
        assert!(
            saved.pinned,
            "a saved search is pinned -- that is the point"
        );
    }

    #[test]
    fn saving_the_same_query_twice_does_not_collide() {
        let mut config = Config::default();
        let first = config.save_filter("has:attach");
        let second = config.save_filter("has:attach");
        assert_ne!(first, second);
        assert_eq!(config.filters.len(), 2);
        assert_eq!(config.filters[&first].query, "has:attach");
        assert_eq!(config.filters[&second].query, "has:attach");
    }

    #[test]
    fn a_query_with_no_alphanumeric_characters_still_gets_a_usable_name() {
        let mut config = Config::default();
        let key = config.save_filter(":::");
        assert_eq!(key, "search");
    }

    #[test]
    fn saving_does_not_touch_a_filter_that_was_already_pinned_false() {
        let mut config = Config::default();
        config.filters.insert(
            "needs-reply".to_string(),
            FilterConfig {
                query: "is:unread from:team".to_string(),
                pinned: false,
                extra: Extras::default(),
            },
        );
        config.save_filter("has:attach");
        assert!(
            !config.filters["needs-reply"].pinned,
            "saving a new filter must not repin an existing one"
        );
    }
}
