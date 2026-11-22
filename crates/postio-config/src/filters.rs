use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Config, Extras, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub pinned: bool,
    /// The key stays a stable TOML-safe identity (#292); this is what the user chose to call it.
    /// `None`
    /// means nobody has renamed it, so it shows as the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `None` sorts after every filter that has one, by key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    /// Never a banner, never a notification.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
    #[serde(flatten)]
    pub extra: Extras,
}

impl Config {
    /// The caller still has to call `patch_filters` afterward; this only touches the in-memory
    /// table.
    /// Returns the key, which is derived rather than chosen.
    pub fn save_filter(&mut self, query: &str) -> String {
        let key = self.unique_filter_key(query);
        self.filters.insert(
            key.clone(),
            FilterConfig {
                query: query.to_string(),
                pinned: true,
                name: None,
                order: None,
                notify: false,
                extra: Extras::default(),
            },
        );
        key
    }

    /// Pinned, it takes the next free place after every pinned filter (the next ⌥ number) and the
    /// others
    /// are given explicit places, as `move_filter` does, so "no order, by key" cannot put it
    /// elsewhere.
    pub fn save_filter_as(
        &mut self,
        query: &str,
        name: Option<&str>,
        pinned: bool,
        notify: bool,
    ) -> String {
        let order = pinned.then(|| {
            let shown = self.ordered_filter_keys();
            for (index, key) in shown.iter().enumerate() {
                if let Some(filter) = self.filters.get_mut(key) {
                    filter.order = Some(index as u32);
                }
            }
            shown.len() as u32
        });
        let key = self.unique_filter_key(query);
        self.filters.insert(
            key.clone(),
            FilterConfig {
                query: query.to_string(),
                pinned,
                name: name.map(str::to_owned),
                order,
                notify,
                extra: Extras::default(),
            },
        );
        key
    }

    /// Renaming to empty text, or to the key itself, is stored as `None`. Returns whether `key`
    /// names a filter.
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

    pub fn delete_filter(&mut self, key: &str) -> bool {
        self.filters.remove(key).is_some()
    }

    /// Leaves its query and place untouched. Returns whether it existed.
    pub fn set_filter_pinned(&mut self, key: &str, pinned: bool) -> bool {
        let Some(filter) = self.filters.get_mut(key) else {
            return false;
        };
        filter.pinned = pinned;
        true
    }

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

    /// Assigns an explicit `order` to every pinned filter, not only the one that moved: an
    /// unordered
    /// filter would keep sorting by key and could land anywhere among the now-explicit ones.
    /// Returns false if `key` is not a pinned filter or was already at the end.
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

    /// A repeated query gets `-2`, `-3` rather than making the user type a name.
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

/// Format-preserving (`toml_edit`): only the `[saved_searches]` table is touched, so comments
/// elsewhere
/// survive (#869). The table is regenerated whole, so a comment on a changed filter entry does not.
pub fn patch_filters(text: &str, filters: &BTreeMap<String, FilterConfig>) -> Result<String> {
    // An empty map serializes as a bare dangling header, so the empty case only removes the
    // section.
    if filters.is_empty() {
        let mut doc = crate::splice::parse(text)?;
        doc.as_table_mut().remove("saved_searches");
        return Ok(doc.to_string());
    }
    crate::splice::replace_section(text, "saved_searches", &FiltersOnly { filters })
}

#[derive(Serialize)]
struct FiltersOnly<'a> {
    #[serde(rename = "saved_searches")]
    filters: &'a BTreeMap<String, FilterConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reorder {
    Up,
    Down,
}

/// A table name has to be a bare TOML key, so anything not ASCII alphanumeric becomes one
/// separator.
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

    #[test]
    fn set_filter_pinned_toggles_an_existing_filter_and_says_whether_it_existed() {
        let mut config = Config::default();
        let key = config.save_filter("is:unread");
        assert!(config.filters[&key].pinned, "save_filter pins on creation");

        assert!(config.set_filter_pinned(&key, false));
        assert!(!config.filters[&key].pinned);

        assert!(config.set_filter_pinned(&key, true));
        assert!(config.filters[&key].pinned);

        assert!(!config.set_filter_pinned("does-not-exist", true));
    }

    #[test]
    fn patch_filters_rewrites_only_the_filters_table_leaving_everything_else_verbatim() {
        let original = "\
# a hand-written comment nobody wants to lose
[ui]
theme = \"dark\" # inline comment, also not to be lost

[saved_searches.old]
query = \"is:unread\"
pinned = true
";
        let mut config = Config::from_toml_str(original).expect("parses");
        config.rename_filter("old", "Unread");

        let patched = patch_filters(original, &config.filters).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "a comment outside [saved_searches] must survive verbatim: {patched}"
        );
        assert!(
            patched.contains("theme = \"dark\" # inline comment, also not to be lost"),
            "an unrelated section's own formatting must survive verbatim: {patched}"
        );

        let reparsed = Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.filters["old"].name.as_deref(), Some("Unread"));
        assert_eq!(reparsed.ui.theme, config.ui.theme);
    }

    #[test]
    fn patch_filters_removes_the_table_entirely_once_the_last_filter_is_deleted() {
        let original = "[saved_searches.old]\nquery = \"is:unread\"\npinned = true\n";
        let mut config = Config::from_toml_str(original).expect("parses");
        config.delete_filter("old");

        let patched = patch_filters(original, &config.filters).expect("patches");
        assert!(
            !patched.contains("[saved_searches"),
            "no filters left means no dangling [saved_searches] header: {patched}"
        );
    }

    #[test]
    fn patch_filters_adds_the_table_when_it_did_not_exist_before() {
        let original = "[ui]\ntheme = \"dark\"\n";
        let mut config = Config::from_toml_str(original).expect("parses");
        config.save_filter("is:flagged");

        let patched = patch_filters(original, &config.filters).expect("patches");
        assert!(patched.contains("[saved_searches."));
        assert!(patched.contains("theme = \"dark\""));

        let reparsed = Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.filters.len(), 1);
    }

    #[test]
    fn an_account_scoped_query_round_trips_through_the_config_file() {
        // ADR 0005 Q5 keeps `account:` in the query language, so a saved search pins itself to an
        // account by carrying the typed text; it must survive the file unchanged.
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

    #[test]
    fn notify_round_trips_and_defaults_to_false() {
        let config =
            Config::from_toml_str("[saved_searches.a]\nquery = \"atlas\"\npinned = true\n")
                .expect("parses");
        assert!(!config.filters["a"].notify, "off unless said");

        let mut filters = config.filters.clone();
        filters.get_mut("a").expect("a").notify = true;
        filters.insert(
            "b".to_owned(),
            FilterConfig {
                query: "harbor".to_owned(),
                pinned: true,
                ..FilterConfig::default()
            },
        );
        let patched = patch_filters("", &filters).expect("patches");
        assert_eq!(
            patched.matches("notify").count(),
            1,
            "only the search that notifies says so:\n{patched}"
        );
        let read = Config::from_toml_str(&patched).expect("parses back");
        assert!(read.filters["a"].notify);
        assert!(!read.filters["b"].notify);
    }

    #[test]
    fn saving_with_a_name_and_switches_takes_the_next_free_place() {
        let mut config = Config::default();
        config.save_filter("from:grace");
        config.save_filter("from:ada");
        let key = config.save_filter_as("atlas budget", Some("Atlas budget from Ada"), true, true);

        let saved = &config.filters[&key];
        assert_eq!(saved.query, "atlas budget");
        assert_eq!(saved.name.as_deref(), Some("Atlas budget from Ada"));
        assert!(saved.pinned && saved.notify);
        assert_eq!(saved.order, Some(2), "after the two already pinned");
        assert_eq!(
            config.ordered_filter_keys(),
            ["from-ada", "from-grace", key.as_str()],
            "the others keep the places they were shown in (by key, unordered)"
        );

        let quiet = config.save_filter_as("harbor", None, false, false);
        assert_eq!(
            config.filters[&quiet].order, None,
            "unpinned takes no place"
        );
        assert!(!config.ordered_filter_keys().contains(&quiet));
    }
}
