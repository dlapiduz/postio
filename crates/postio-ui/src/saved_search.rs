//! Saved searches in `[saved_searches]` and the four verbs a sidebar offers: save, rename, move,
//! remove (#1574).
//! The file is read fresh on every verb and only that table is spliced, so a stale read cannot
//! erase hand edits (#885).
//! A file that does not parse is refused, not defaulted: the default's empty table would be written
//! over the user's searches.

use std::path::Path;

use postio_config::{Config, ConfigError};

pub use postio_config::filters::Reorder;

/// Unpacked from `FilterConfig`, whose name is a map key, so frontends get a flat row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// The stable identity a rename, reorder or delete acts on (#292); never shown.
    pub key: String,
    pub name: String,
    pub query: String,
    pub notify: bool,
}

/// One verb type so reading, patching and writing back is written once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb<'a> {
    /// `Verb::save` is the plain form a sidebar's Save makes. Blank does nothing: pinning an empty
    /// query pins "everything".
    Save {
        /// As it stands in the search field, not as last run.
        query: &'a str,
        name: Option<&'a str>,
        pin: bool,
        notify: bool,
        dates: Dates,
    },
    Rename {
        key: &'a str,
        /// The new label. Blank, or the key repeated, means "not renamed".
        name: &'a str,
    },
    Move {
        key: &'a str,
        direction: Reorder,
    },
    Delete {
        key: &'a str,
    },
}

/// Both are one query language, so only the string says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dates {
    AsTyped,
    Rolling { today: chrono::NaiveDate },
    Fixed { today: chrono::NaiveDate },
}

impl<'a> Verb<'a> {
    pub fn save(query: &'a str) -> Self {
        Verb::Save {
            query,
            name: None,
            pin: true,
            notify: false,
            dates: Dates::AsTyped,
        }
    }
}

/// Always returned, even for a no-op (`changed: None`, the list as it stood), so the caller draws
/// the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub searches: Vec<SavedSearch>,
    /// `None` when nothing changed: blank query, unknown key, or a row already at the end.
    /// Callers keep the keyboard on the row that moved.
    pub changed: Option<String>,
}

/// Shared wording, so a destructive confirmation reads the same on every platform (ADR 0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    pub title: &'static str,
    pub body: Option<&'static str>,
    pub confirm: &'static str,
    pub cancel: &'static str,
}

/// A config edit has no undo stack, so deleting is confirmed (#292).
pub const DELETE_PROMPT: Prompt = Prompt {
    title: "Delete this saved search?",
    body: Some("It can be saved again from the same query, but the query itself is gone."),
    confirm: "Delete",
    cancel: "Keep",
};

/// No body; cancel says "Cancel" since nothing is taken away.
pub const RENAME_PROMPT: Prompt = Prompt {
    title: "Rename this saved search?",
    body: None,
    confirm: "Rename",
    cancel: "Cancel",
};

/// In `ordered_filter_keys` order (explicit `order`, then by key), so frontends do not re-sort and
/// disagree (#292).
pub fn pinned(config: &Config) -> Vec<SavedSearch> {
    config
        .ordered_filter_keys()
        .into_iter()
        .filter_map(|key| {
            let filter = config.filters.get(&key)?;
            Some(SavedSearch {
                name: filter.name.clone().unwrap_or_else(|| key.clone()),
                query: filter.query.clone(),
                notify: filter.notify,
                key,
            })
        })
        .collect()
}

/// Best effort: a missing or unparseable file shows the built-in defaults. The errors that matter
/// come from `apply`.
pub fn load(path: &Path) -> Vec<SavedSearch> {
    Config::load_from_path(path)
        .map(|config| pinned(&config))
        .unwrap_or_default()
}

/// The whole rule with the I/O lifted out; `apply` adds the read and write.
pub fn edit(text: &str, verb: Verb<'_>) -> postio_config::Result<Edit> {
    let mut config = Config::from_toml_str(text)?;
    let before = config.filters.clone();

    // Each arm says whether it named something real; the comparison below decides whether anything
    // changed, so a byte-identical rewrite never happens.
    let named = match verb {
        Verb::Save {
            query,
            name,
            pin,
            notify,
            dates,
        } => {
            let query = match dates {
                Dates::AsTyped => query.trim().to_owned(),
                Dates::Rolling { today } | Dates::Fixed { today } => postio_search::edit::apply(
                    query,
                    postio_search::edit::Edit::Dates {
                        rolling: matches!(dates, Dates::Rolling { .. }),
                    },
                    today,
                ),
            };
            let name = name.map(str::trim).filter(|name| !name.is_empty());
            (!query.is_empty()).then(|| config.save_filter_as(&query, name, pin, notify))
        }
        Verb::Rename { key, name } => config.rename_filter(key, name).then(|| key.to_owned()),
        Verb::Move { key, direction } => config.move_filter(key, direction).then(|| key.to_owned()),
        Verb::Delete { key } => config.delete_filter(key).then(|| key.to_owned()),
    };

    let searches = pinned(&config);
    let Some(changed) = named.filter(|_| config.filters != before) else {
        return Ok(Edit {
            text: text.to_owned(),
            searches,
            changed: None,
        });
    };

    Ok(Edit {
        text: postio_config::filters::patch_filters(text, &config.filters)?,
        searches,
        changed: Some(changed),
    })
}

/// A missing file is empty: the first saved search creates `config.toml`.
pub fn apply(path: &Path, verb: Verb<'_>) -> postio_config::Result<Edit> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        // A file that exists but cannot be read is not an empty one: writing would clobber it.
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let edit = edit(&text, verb)?;
    if edit.changed.is_some() {
        // Atomic: the watcher sees a rename like any editor save, and a failed in-place write would
        // truncate a hand-edited file.
        postio_config::save::write_atomically(path, &edit.text).map_err(|source| {
            ConfigError::Write {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(edit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[saved_searches.urgent]
query = \"is:unread\"
pinned = true

[ui]
density = \"compact\"
some_future_key = 42
";

    /// Three rows, so "one place" and "either end" differ: a single row is both first and last.
    const THREE: &str = "\
[saved_searches.a]
query = \"from:ada\"
pinned = true

[saved_searches.b]
query = \"from:grace\"
pinned = true

[saved_searches.c]
query = \"from:alan\"
pinned = true
";

    #[test]
    fn saving_a_query_pins_it_and_names_it_from_its_own_text() {
        let edit = edit("", Verb::save("is:unread from:team")).expect("an empty file");

        assert_eq!(
            edit.changed.as_deref(),
            Some("is-unread-from-team"),
            "the key is derived from the query, and the caller needs it"
        );
        assert_eq!(
            edit.searches,
            vec![SavedSearch {
                key: "is-unread-from-team".to_owned(),
                name: "is-unread-from-team".to_owned(),
                query: "is:unread from:team".to_owned(),
                notify: false,
            }],
            "a search nobody has renamed draws under its key"
        );
        assert!(
            edit.text.contains("[saved_searches.is-unread-from-team]"),
            "the table did not reach the file:\n{}",
            edit.text
        );
    }

    #[test]
    fn saving_nothing_saves_nothing() {
        let edit = edit(SAMPLE, Verb::save("   ")).expect("the sample parses");

        assert_eq!(edit.changed, None);
        assert_eq!(edit.text, SAMPLE, "a no-op must not rewrite the file");
        assert_eq!(
            edit.searches.len(),
            1,
            "the list still has to be drawable: {:?}",
            edit.searches
        );
    }

    #[test]
    fn a_saved_search_draws_under_the_name_it_was_given() {
        let renamed = edit(
            SAMPLE,
            Verb::Rename {
                key: "urgent",
                name: "Needs a reply",
            },
        )
        .expect("the sample parses");

        assert_eq!(renamed.changed.as_deref(), Some("urgent"));
        assert_eq!(
            renamed.searches[0].name, "Needs a reply",
            "the label follows the rename"
        );
        assert_eq!(
            renamed.searches[0].key, "urgent",
            "the key is the identity and a rename must not move it (#292)"
        );
    }

    #[test]
    fn renaming_to_nothing_puts_the_key_back_as_the_label() {
        let named = edit(
            SAMPLE,
            Verb::Rename {
                key: "urgent",
                name: "Needs a reply",
            },
        )
        .expect("the sample parses");
        let cleared = edit(
            &named.text,
            Verb::Rename {
                key: "urgent",
                name: "  ",
            },
        )
        .expect("the renamed file parses");

        assert_eq!(
            cleared.searches[0].name, "urgent",
            "clearing a name is not an empty label, it is no label"
        );
    }

    #[test]
    fn renaming_a_search_that_is_not_there_changes_nothing() {
        let edit = edit(
            SAMPLE,
            Verb::Rename {
                key: "no-such-search",
                name: "Whatever",
            },
        )
        .expect("the sample parses");

        assert_eq!(edit.changed, None);
        assert_eq!(edit.text, SAMPLE);
    }

    #[test]
    fn a_reorder_moves_one_place_and_writes_the_whole_order_down() {
        let moved = edit(
            THREE,
            Verb::Move {
                key: "c",
                direction: Reorder::Up,
            },
        )
        .expect("the file parses");

        assert_eq!(moved.changed.as_deref(), Some("c"));
        let keys: Vec<&str> = moved.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["a", "c", "b"]);

        let reread = edit(&moved.text, Verb::save("")).expect("the patched file parses");
        let keys: Vec<&str> = reread.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["a", "c", "b"], "the order did not survive the write");
    }

    #[test]
    fn a_reorder_down_walks_the_other_way() {
        let moved = edit(
            THREE,
            Verb::Move {
                key: "a",
                direction: Reorder::Down,
            },
        )
        .expect("the file parses");

        assert_eq!(moved.changed.as_deref(), Some("a"));
        let keys: Vec<&str> = moved.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            ["b", "a", "c"],
            "`a` went down exactly one place -- not up, and not to the end"
        );

        let reread = edit(&moved.text, Verb::save("")).expect("the patched file parses");
        let keys: Vec<&str> = reread.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "c"], "the order did not survive the write");
    }

    #[test]
    fn a_row_at_the_end_it_is_moving_toward_stays_where_it_is() {
        for (key, direction) in [("a", Reorder::Up), ("c", Reorder::Down)] {
            let refused = edit(THREE, Verb::Move { key, direction }).expect("the file parses");
            assert_eq!(
                refused.changed, None,
                "{key} had nowhere to go and reported that it moved"
            );
            assert_eq!(refused.text, THREE, "a refusal rewrote the file");
        }

        for (key, direction) in [("a", Reorder::Down), ("c", Reorder::Up)] {
            let moved = edit(THREE, Verb::Move { key, direction }).expect("the file parses");
            assert_eq!(
                moved.changed.as_deref(),
                Some(key),
                "{key} refused the direction it had room for"
            );
        }
    }

    #[test]
    fn renaming_a_search_to_the_name_it_already_has_writes_nothing() {
        // `rename_filter` says yes to a rename to the stored name; comparing against the table as
        // it was keeps a no-op from rewriting the file.
        let named = edit(
            SAMPLE,
            Verb::Rename {
                key: "urgent",
                name: "Needs a reply",
            },
        )
        .expect("the sample parses");
        assert_eq!(named.changed.as_deref(), Some("urgent"));

        let again = edit(
            &named.text,
            Verb::Rename {
                key: "urgent",
                name: "Needs a reply",
            },
        )
        .expect("the renamed file parses");

        assert_eq!(
            again.changed, None,
            "renaming to the name it already has is not a change"
        );
        assert_eq!(again.text, named.text, "and it must not rewrite the file");
        assert_eq!(
            again.searches[0].name, "Needs a reply",
            "the list is still drawable either way"
        );
    }

    #[test]
    fn deleting_takes_the_row_and_its_table_away() {
        let gone = edit(SAMPLE, Verb::Delete { key: "urgent" }).expect("the sample parses");

        assert_eq!(gone.changed.as_deref(), Some("urgent"));
        assert!(gone.searches.is_empty());
        assert!(
            !gone.text.contains("[saved_searches.urgent]"),
            "the table is still in the file:\n{}",
            gone.text
        );
    }

    #[test]
    fn deleting_one_that_is_not_there_changes_nothing() {
        let edit = edit(
            SAMPLE,
            Verb::Delete {
                key: "no-such-search",
            },
        )
        .expect("the sample parses");

        assert_eq!(edit.changed, None);
        assert_eq!(edit.text, SAMPLE);
    }

    #[test]
    fn an_unpinned_filter_is_not_a_sidebar_row() {
        let text = "\
[saved_searches.hidden]
query = \"is:unread\"
pinned = false
";
        let edit = edit(text, Verb::save("has:attach")).expect("the file parses");

        let keys: Vec<&str> = edit.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["has-attach"]);
    }

    #[test]
    fn an_edit_leaves_every_other_line_in_the_file_alone() {
        // Spliced via `patch_filters`, not reserialized: the settings file is not ours to reformat
        // (#885).
        let saved = edit(SAMPLE, Verb::save("has:attach")).expect("the sample parses");

        assert!(
            saved
                .text
                .contains("# hand-written, and it should stay that way"),
            "the comment did not survive:\n{}",
            saved.text
        );
        assert!(
            saved.text.contains("idle = true"),
            "[sync] did not survive:\n{}",
            saved.text
        );
        assert!(
            saved.text.contains("some_future_key = 42"),
            "a key this build does not know was dropped:\n{}",
            saved.text
        );
        assert!(
            saved.text.contains("[saved_searches.has-attach]"),
            "and the edit itself still has to land:\n{}",
            saved.text
        );
    }

    #[test]
    fn a_file_that_will_not_parse_is_refused_rather_than_emptied() {
        // A broken file must not default: the defaults have no filters and the patch would erase
        // the user's searches.
        let broken = "[ui]\ndensity = 42\n\n[saved_searches.keep]\nquery = \"is:unread\"\n";

        assert!(
            edit(broken, Verb::save("has:attach")).is_err(),
            "a config that does not parse must not be rewritten from defaults"
        );
        assert!(edit("this is not toml {{{", Verb::Delete { key: "keep" }).is_err());
    }

    #[test]
    fn the_file_is_written_only_when_something_changed() {
        let dir = std::env::temp_dir().join(format!(
            "postio-saved-search-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("config.toml");

        let saved = apply(&path, Verb::save("is:unread")).expect("a first save");
        assert_eq!(saved.changed.as_deref(), Some("is-unread"));
        assert_eq!(load(&path), saved.searches, "the file is the sidebar");

        let before = std::fs::metadata(&path).expect("the file exists");
        let nothing = apply(
            &path,
            Verb::Delete {
                key: "no-such-search",
            },
        )
        .expect("a no-op");
        assert_eq!(nothing.changed, None);
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            saved.text,
            "a verb that changed nothing rewrote the file anyway"
        );
        assert_eq!(
            before.len(),
            std::fs::metadata(&path).expect("the file exists").len()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_questions_are_asked_in_words_both_frontends_share() {
        assert_eq!(DELETE_PROMPT.title, "Delete this saved search?");
        assert!(
            DELETE_PROMPT
                .body
                .is_some_and(|body| body.contains("query"))
        );
        assert_eq!(DELETE_PROMPT.confirm, "Delete");
        assert_eq!(RENAME_PROMPT.body, None);
    }

    #[test]
    fn saving_with_a_name_pin_and_notify_writes_all_four_and_the_next_place() {
        let edit = edit(
            THREE,
            Verb::Save {
                query: "atlas budget",
                name: Some("Atlas budget from Ada"),
                pin: true,
                notify: true,
                dates: Dates::AsTyped,
            },
        )
        .expect("three parses");
        let key = edit.changed.clone().expect("saved");
        let saved = edit.searches.last().expect("listed");
        assert_eq!(saved.key, key, "after the three already pinned");
        assert_eq!(saved.name, "Atlas budget from Ada");
        assert!(saved.notify);
        let read = Config::from_toml_str(&edit.text).expect("parses back");
        let filter = &read.filters[&key];
        assert_eq!(filter.query, "atlas budget");
        assert_eq!(filter.name.as_deref(), Some("Atlas budget from Ada"));
        assert!(filter.pinned && filter.notify);
        assert_eq!(filter.order, Some(3), "the next free place: ⌥4");
    }

    #[test]
    fn a_rolling_save_keeps_its_dates_relative_and_a_fixed_one_iso() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let saved = |query: &str, dates| {
            let edit = edit(
                "",
                Verb::Save {
                    query,
                    name: None,
                    pin: true,
                    notify: false,
                    dates,
                },
            )
            .expect("an empty file");
            edit.searches[0].query.clone()
        };
        assert_eq!(
            saved("atlas after:2026-07-01", Dates::Rolling { today }),
            "atlas after:90d",
            "always the last 90 days"
        );
        assert_eq!(
            saved("atlas after:90d", Dates::Fixed { today }),
            "atlas after:2026-07-01",
            "always since 1 July"
        );
        assert_eq!(
            saved("atlas after:jul1", Dates::AsTyped),
            "atlas after:jul1"
        );
    }
}
