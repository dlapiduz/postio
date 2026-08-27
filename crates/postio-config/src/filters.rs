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
    /// A display name distinct from the `[filters.<key>]` key -- the key
    /// stays a stable, TOML-safe identity (issue #292); this is what the
    /// user actually chose to call it, in whatever text they typed.
    ///
    /// `None` means "nobody has renamed this yet", so it shows as the key
    /// still does -- every filter saved before renaming existed, and one
    /// saved today, both start this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where this filter sits among the others in the sidebar, lowest
    /// first.
    ///
    /// `None` sorts after every filter that has one, by key -- the
    /// alphabetical order every filter already had before reordering
    /// existed, so a file nobody has reordered behaves exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
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
                name: None,
                order: None,
                extra: Extras::default(),
            },
        );
        key
    }

    /// Give `key`'s saved search a display name distinct from its
    /// `[filters.<key>]` key.
    ///
    /// Renaming to empty text, or to text that just repeats the key, is
    /// stored the same way as never having renamed it -- `None`, not a name
    /// that would show identically to the key it already falls back to.
    ///
    /// Returns whether `key` names a filter at all.
    pub fn rename_filter(&mut self, key: &str, name: &str) -> bool {
        let Some(filter) = self.filters.get_mut(key) else {
            return false;
        };
        let name = name.trim();
        filter.name = if name.is_empty() || name == key {
            None
        } else {
            Some(name.to_owned())
        };
        true
    }

    /// Remove `key`'s saved search entirely. Returns whether it existed.
    pub fn delete_filter(&mut self, key: &str) -> bool {
        self.filters.remove(key).is_some()
    }

    /// Every *pinned* filter's key, in the order the sidebar shows them:
    /// explicit [`FilterConfig::order`] first (lowest first), then anything
    /// without one, alphabetically by key.
    pub fn ordered_filter_keys(&self) -> Vec<String> {
        let mut keys: Vec<&String> = self
            .filters
            .iter()
            .filter(|(_, filter)| filter.pinned)
            .map(|(key, _)| key)
            .collect();
        keys.sort_by(|a, b| {
            let order_of = |key: &str| self.filters[key].order;
            match (order_of(a), order_of(b)) {
                (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.cmp(b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(b),
            }
        });
        keys.into_iter().cloned().collect()
    }

    /// Move `key` one place earlier or later among the pinned filters, per
    /// [`Config::ordered_filter_keys`].
    ///
    /// Assigns an explicit `order` to *every* pinned filter as a side
    /// effect, not only the one that moved: the moment one filter's
    /// position is pinned down, "no explicit order, alphabetical" stops
    /// being a coherent story for the ones left without one -- an
    /// unordered filter would keep sorting by key, which could put it
    /// anywhere at all among the now-explicit ones rather than where it
    /// visibly sat a moment ago.
    ///
    /// Returns whether `key` actually moved -- false if it does not name a
    /// pinned filter, or if it was already at the end being moved toward.
    pub fn move_filter(&mut self, key: &str, direction: Reorder) -> bool {
        let mut ordered = self.ordered_filter_keys();
        let Some(at) = ordered.iter().position(|k| k == key) else {
            return false;
        };
        let swap_with = match direction {
            Reorder::Up => at.checked_sub(1),
            Reorder::Down => (at + 1 < ordered.len()).then_some(at + 1),
        };
        let Some(swap_with) = swap_with else {
            return false;
        };
        ordered.swap(at, swap_with);
        for (index, key) in ordered.iter().enumerate() {
            self.filters
                .get_mut(key)
                .expect("ordered_filter_keys only returns real keys")
                .order = Some(index as u32);
        }
        true
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

/// Which way [`Config::move_filter`] should walk a saved search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reorder {
    /// Toward the front of the sidebar list.
    Up,
    /// Toward the back.
    Down,
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
                ..Default::default()
            },
        );
        config.save_filter("has:attach");
        assert!(
            !config.filters["needs-reply"].pinned,
            "saving a new filter must not repin an existing one"
        );
    }

    // -- Acceptance: rename, reorder, delete (#292) ------------------------

    #[test]
    fn renaming_a_filter_sets_a_display_name_distinct_from_its_key() {
        let mut config = Config::default();
        let key = config.save_filter("is:unread from:team");
        assert!(config.rename_filter(&key, "Needs a reply"));
        assert_eq!(config.filters[&key].name.as_deref(), Some("Needs a reply"));
        assert_eq!(
            config.filters[&key].query, "is:unread from:team",
            "renaming must not touch the query the filter runs"
        );
    }

    #[test]
    fn renaming_an_unknown_key_does_nothing_and_says_so() {
        let mut config = Config::default();
        assert!(!config.rename_filter("does-not-exist", "Anything"));
    }

    #[test]
    fn renaming_to_empty_or_to_the_key_itself_clears_the_name() {
        let mut config = Config::default();
        let key = config.save_filter("has:attach");
        config.rename_filter(&key, "Attachments");
        assert!(config.filters[&key].name.is_some());

        config.rename_filter(&key, "   ");
        assert_eq!(
            config.filters[&key].name, None,
            "blank text is not a name -- it is giving the key back"
        );

        config.rename_filter(&key, "Attachments");
        config.rename_filter(&key, &key);
        assert_eq!(
            config.filters[&key].name, None,
            "a name identical to the key is indistinguishable from never \
             having renamed it, so it is stored the same way"
        );
    }

    #[test]
    fn deleting_a_filter_removes_it_and_says_whether_it_was_there() {
        let mut config = Config::default();
        let key = config.save_filter("has:attach");
        assert!(config.delete_filter(&key));
        assert!(!config.filters.contains_key(&key));
        assert!(
            !config.delete_filter(&key),
            "deleting it again has nothing left to remove"
        );
    }

    #[test]
    fn ordered_filter_keys_puts_explicit_order_first_then_falls_back_to_the_key() {
        let mut config = Config::default();
        let unordered_b = config.save_filter("subject:b");
        let unordered_a = config.save_filter("subject:a");
        let first = config.save_filter("subject:first");
        let second = config.save_filter("subject:second");
        config.filters.get_mut(&first).unwrap().order = Some(0);
        config.filters.get_mut(&second).unwrap().order = Some(1);

        assert_eq!(
            config.ordered_filter_keys(),
            vec![first, second, unordered_a, unordered_b],
            "explicit order wins, in order; everything else falls back to \
             alphabetical by key"
        );
    }

    #[test]
    fn ordered_filter_keys_ignores_filters_that_are_not_pinned() {
        let mut config = Config::default();
        let pinned = config.save_filter("is:flagged");
        config.filters.insert(
            "unpinned".to_string(),
            FilterConfig {
                query: "is:unread".to_string(),
                pinned: false,
                ..Default::default()
            },
        );
        assert_eq!(config.ordered_filter_keys(), vec![pinned]);
    }

    #[test]
    fn moving_a_filter_up_swaps_it_with_its_neighbour() {
        let mut config = Config::default();
        let a = config.save_filter("subject:a");
        let b = config.save_filter("subject:b");
        let c = config.save_filter("subject:c");
        assert_eq!(
            config.ordered_filter_keys(),
            vec![a.clone(), b.clone(), c.clone()]
        );

        assert!(config.move_filter(&c, Reorder::Up));
        assert_eq!(
            config.ordered_filter_keys(),
            vec![a.clone(), c.clone(), b.clone()],
            "c should have swapped with b, the one place ahead of it"
        );

        assert!(config.move_filter(&c, Reorder::Up));
        assert_eq!(
            config.ordered_filter_keys(),
            vec![c, a, b],
            "and again to the very front"
        );
    }

    #[test]
    fn moving_a_filter_past_either_end_does_nothing_and_says_so() {
        let mut config = Config::default();
        let a = config.save_filter("subject:a");
        let b = config.save_filter("subject:b");

        assert!(!config.move_filter(&a, Reorder::Up), "already at the front");
        assert!(
            !config.move_filter(&b, Reorder::Down),
            "already at the back"
        );
        assert_eq!(config.ordered_filter_keys(), vec![a, b]);
    }

    #[test]
    fn moving_an_unpinned_or_unknown_filter_does_nothing() {
        let mut config = Config::default();
        config.save_filter("subject:a");
        assert!(!config.move_filter("does-not-exist", Reorder::Down));
    }

    #[test]
    fn once_anything_is_reordered_every_pinned_filter_gets_an_explicit_order() {
        // The rationale in `move_filter`'s own doc: leaving some filters on
        // the alphabetical fallback once another has an explicit position
        // is a trap, so a move touches everyone's `order`, not just the
        // pair that swapped.
        let mut config = Config::default();
        let a = config.save_filter("subject:a");
        let b = config.save_filter("subject:b");
        let c = config.save_filter("subject:c");

        config.move_filter(&b, Reorder::Up);

        for key in [&a, &b, &c] {
            assert!(
                config.filters[key].order.is_some(),
                "{key} should have an explicit order after any move"
            );
        }
    }
    // -- Acceptance: `account:` survives the config file (#186) -------------

    #[test]
    fn an_account_scoped_query_round_trips_through_the_config_file() {
        // ADR 0005 Q5 keeps `account:` in the *query language* rather than in
        // a config field of its own, so a saved search pins itself to an
        // account by carrying the text the user typed. That only works if the
        // text survives the file unchanged -- a colon-bearing value is
        // exactly the kind of thing a serializer quotes, re-quotes, or
        // mangles.
        //
        // This crate never parses the query (it does not depend on
        // `postio-search` and should not), so "unchanged" is the whole
        // contract it can offer, and the whole contract the executor needs.
        let mut config = Config::default();
        let key = config.save_filter(r#"account:"Work Mail" is:unread"#);

        let written = toml::to_string(&config).expect("serializes");
        let read = Config::from_toml_str(&written).expect("parses back");

        assert_eq!(
            read.filters[&key].query, r#"account:"Work Mail" is:unread"#,
            "the saved search came back changed, so it no longer names the \
             account it was pinned to: {written}"
        );
    }
}
