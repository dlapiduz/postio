//! Keeping a query: `[filters]`, and the four verbs a sidebar offers over one.
//!
//! Half of what makes Postio's search worth learning is that a query can
//! become a folder — `docs/PRODUCT.md` puts finding things among the three
//! jobs this application has to beat the alternatives at, and a search you
//! have to retype is a search you stop composing carefully. The promise is
//! four edits to `config.toml`: save the query that is showing, rename the
//! row it made, move it among its neighbours, remove it.
//!
//! None of those four is a widget. All four lived in `postio-gtk::config`
//! anyway — read the file fresh, mutate the `[filters]` table, patch it back,
//! repaint the sidebar — which is why the macOS search surface could draw a
//! *Save search as folder* affordance, leave it enabled, and have it do
//! nothing (#1574). A frontend that cannot reach a rule re-invents it or goes
//! without, and going without is the quieter of the two failures.
//!
//! # Why the file is read fresh on every verb
//!
//! Not out of a shared `Config` the application is already holding. A saved
//! search is a `config.toml` edit, and `config.toml` is a file a person edits
//! by hand and an editor rewrites from underneath us; the copy in memory is a
//! snapshot of whenever it was last loaded. Writing a patched version of a
//! *stale* read is how a hand-written `[sync]` block disappears an hour after
//! it was typed. Reading immediately before patching keeps the window in
//! which that can happen down to this function.
//!
//! # Why only `[filters]` is rewritten
//!
//! Through [`postio_config::filters::patch_filters`], which is a
//! `toml_edit` splice of one table, never a reserialize of the whole
//! `Config`. The rest of the file — comments, key order, the tables this
//! version of Postio has never heard of — comes back byte for byte. #885 is
//! what happens otherwise: every saved search silently reformatting somebody's
//! settings file.
//!
//! # Why a file that will not parse is refused rather than defaulted
//!
//! [`edit`] returns the parse error instead of falling back to
//! [`Config::default`]. The fallback is the tempting reading — a broken file
//! means we know nothing, so start from nothing — and it is wrong in exactly
//! this one place: "nothing" includes an empty `[filters]` table, so the very
//! next `patch_filters` writes that emptiness over the searches the user
//! still has. A file that does not parse is a file to leave alone.

use std::path::Path;

use postio_config::{Config, ConfigError};

pub use postio_config::filters::Reorder;

/// One pinned `[filters]` entry, as a sidebar draws it.
///
/// Not `postio_config::FilterConfig`, whose name is a map key rather than a
/// field: a frontend takes a flat list of rows to draw, so the key/value
/// split of the config schema is unpacked here once instead of in each
/// frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// The `[filters.<key>]` key — the stable identity a rename, a reorder or
    /// a delete acts on (#292). Never shown; [`SavedSearch::name`] draws.
    pub key: String,
    /// What the row shows: the display name the user chose, or the key when
    /// nobody has renamed it.
    pub name: String,
    /// The query text this row runs when it is picked.
    pub query: String,
}

/// One of the four things a sidebar can do to the saved searches.
///
/// A verb rather than four functions so that reading the file, patching it
/// and writing it back is written once. The four differ only in which
/// `postio_config` mutation they make, and every one of them has to make the
/// same promises about the rest of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb<'a> {
    /// Keep `query` as a new pinned search, named from its own text.
    ///
    /// Blank does nothing: pinning an empty query pins "everything", which is
    /// not a folder anybody meant to make.
    Save {
        /// The query as it stands in the search field, not as it was last
        /// run — saving what is on screen is the gesture.
        query: &'a str,
    },
    /// Give the search under `key` a display name of its own.
    Rename {
        /// Which search.
        key: &'a str,
        /// The new label. Blank, or the key repeated, means "not renamed".
        name: &'a str,
    },
    /// Move the search under `key` one place among its neighbours.
    Move {
        /// Which search.
        key: &'a str,
        /// Which way.
        direction: Reorder,
    },
    /// Remove the search under `key` entirely.
    Delete {
        /// Which search.
        key: &'a str,
    },
}

/// What a verb left behind.
///
/// Always returned, even when the verb did nothing, because the caller's next
/// move is the same either way: draw [`Edit::searches`]. A no-op is
/// `changed: None` and the list as it already stood, not an error and not an
/// absence the frontend has to invent a list for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The file as it now reads. Unchanged text when nothing happened.
    pub text: String,
    /// The rows the sidebar should draw now, in order.
    pub searches: Vec<SavedSearch>,
    /// The key of the entry that actually changed, or `None` when nothing
    /// did — a blank query, a key that names no search, a row already at the
    /// end it was being moved toward.
    ///
    /// Which row moved is not a nicety: after a reorder the keyboard has to
    /// stay on the row it moved rather than on the position it left, and
    /// after a save the new row is the one worth showing.
    pub changed: Option<String>,
}

/// The words a frontend asks a question in.
///
/// Wording crosses because wording drifts — ADR 0019 Q6's whole argument —
/// and a destructive confirmation is the worst place for two platforms to
/// have written their own sentence. Saved searches are the first two
/// questions to cross; the shape is deliberately general enough for the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    /// The question itself.
    pub title: &'static str,
    /// What it costs, when that is not obvious from the title.
    pub body: Option<&'static str>,
    /// The button that goes through with it.
    pub confirm: &'static str,
    /// The button that does not.
    pub cancel: &'static str,
}

/// Asked before a saved search is deleted.
///
/// A config-file edit has no undo stack to reach — #292 weighed that directly
/// — so `CommandId::DeleteSavedSearch` declares `Recovery::Confirm` and this
/// is the confirmation. The body says what is actually lost: not the folder,
/// which can be made again, but the query somebody composed.
pub const DELETE_PROMPT: Prompt = Prompt {
    title: "Delete this saved search?",
    body: Some("It can be saved again from the same query, but the query itself is gone."),
    confirm: "Delete",
    cancel: "Keep",
};

/// Asked when a saved search is being renamed.
///
/// No body: the entry, pre-filled with the name showing now, says everything
/// a sentence would. The cancel is "Cancel" rather than [`DELETE_PROMPT`]'s
/// "Keep" because there is nothing being taken away to keep.
pub const RENAME_PROMPT: Prompt = Prompt {
    title: "Rename this saved search?",
    body: None,
    confirm: "Rename",
    cancel: "Cancel",
};

/// The pinned entries of `config`'s `[filters]`, in the order a sidebar shows
/// them.
///
/// [`Config::ordered_filter_keys`]'s order, which is explicit `order` first
/// and then alphabetically by key — so a frontend draws the list exactly as
/// given rather than sorting it again and disagreeing (#292).
pub fn pinned(config: &Config) -> Vec<SavedSearch> {
    config
        .ordered_filter_keys()
        .into_iter()
        .filter_map(|key| {
            let filter = config.filters.get(&key)?;
            Some(SavedSearch {
                name: filter.name.clone().unwrap_or_else(|| key.clone()),
                query: filter.query.clone(),
                key,
            })
        })
        .collect()
}

/// The saved searches in the file at `path`, for the sidebar's first draw.
///
/// Best effort, the way the running application already treats a broken
/// `config.toml`: a missing file has no searches in it, and one that will not
/// parse is a reason to show the built-in defaults rather than to refuse to
/// draw a sidebar. The errors that matter are the ones [`apply`] returns,
/// where a write was about to happen.
pub fn load(path: &Path) -> Vec<SavedSearch> {
    Config::load_from_path(path)
        .map(|config| pinned(&config))
        .unwrap_or_default()
}

/// Carry out `verb` against `text`, touching no file.
///
/// The whole rule, with the I/O lifted out so it can be proven at the cheapest
/// layer there is. [`apply`] is this plus a read and a write.
pub fn edit(text: &str, verb: Verb<'_>) -> postio_config::Result<Edit> {
    let mut config = Config::from_toml_str(text)?;
    let before = config.filters.clone();

    // Each arm answers "did this name something real", which is not the same
    // question as "did anything change" -- renaming a search to the name it
    // already had names something real and changes nothing. The comparison
    // below settles the second question for all four at once, so no verb has
    // to remember to, and a write that would rewrite the file byte for byte
    // never happens.
    let named = match verb {
        Verb::Save { query } => {
            let query = query.trim();
            (!query.is_empty()).then(|| config.save_filter(query))
        }
        Verb::Rename { key, name } => config.rename_filter(key, name).then(|| key.to_owned()),
        Verb::Move { key, direction } => config.move_filter(key, direction).then(|| key.to_owned()),
        Verb::Delete { key } => config.delete_filter(key).then(|| key.to_owned()),
    };

    // Computed from the mutated config either way: when nothing changed the
    // two are equal by construction, and the caller still needs a list to
    // draw rather than an absence to interpret.
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

/// Carry out `verb` against the file at `path`, writing it back if anything
/// changed.
///
/// A missing file is an empty one — the first saved search on a machine that
/// has never opened the settings panel creates `config.toml`, the same as any
/// other first write.
pub fn apply(path: &Path, verb: Verb<'_>) -> postio_config::Result<Edit> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        // Distinguished from "not there" on purpose. A file that exists and
        // cannot be read -- a permission that changed, a disk that went away
        // -- must not be treated as an empty one, because the next step would
        // write a fresh `config.toml` over whatever is actually there.
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let edit = edit(&text, verb)?;
    if edit.changed.is_some() {
        // Atomically, which is `postio_config::save`'s whole contract with
        // the watcher: a running Postio learns about its own settings change
        // the same way it learns about one made in `$EDITOR`, because both
        // arrive as a rename over the file rather than as a write into it.
        // The safety half matters more here than the liveness half -- this
        // file is hand-edited, and a write that fails part way through an
        // in-place rewrite leaves it truncated.
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

    /// A file with the things a careless rewrite destroys: a comment, tables
    /// either side of `[filters]`, and a key nothing in this build reads.
    const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[filters.urgent]
query = \"is:unread\"
pinned = true

[ui]
density = \"compact\"
some_future_key = 42
";

    /// Three pinned rows, because "one place" and "either end" are only
    /// different questions when there is a middle.
    ///
    /// [`SAMPLE`] has one filter, and that single row is simultaneously the
    /// first and the last: a reorder over it refuses whichever way it is
    /// asked, so it cannot tell a direction that was read from one that was
    /// thrown away.
    const THREE: &str = "\
[filters.a]
query = \"from:ada\"
pinned = true

[filters.b]
query = \"from:grace\"
pinned = true

[filters.c]
query = \"from:alan\"
pinned = true
";

    #[test]
    fn saving_a_query_pins_it_and_names_it_from_its_own_text() {
        let edit = edit(
            "",
            Verb::Save {
                query: "is:unread from:team",
            },
        )
        .expect("an empty file");

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
            }],
            "a search nobody has renamed draws under its key"
        );
        assert!(
            edit.text.contains("[filters.is-unread-from-team]"),
            "the table did not reach the file:\n{}",
            edit.text
        );
    }

    #[test]
    fn saving_nothing_saves_nothing() {
        // Pinning an empty query pins "everything", which is not a folder
        // anyone meant to make -- and it would still take a row in the
        // sidebar and a line in the file.
        let edit = edit(SAMPLE, Verb::Save { query: "   " }).expect("the sample parses");

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
        // Moving one row pins every row's position: leaving the others
        // implicit means they keep sorting alphabetically, which can put
        // them anywhere at all relative to the one that now has a number.
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

        // And it is durable: re-reading the patched text has to give the same
        // order, or the row springs back the next time anything reloads.
        let reread = edit(&moved.text, Verb::Save { query: "" }).expect("the patched file parses");
        let keys: Vec<&str> = reread.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["a", "c", "b"], "the order did not survive the write");
    }

    #[test]
    fn a_reorder_down_walks_the_other_way() {
        // The two directions are one enum variant apart and are wired to two
        // separate commands, which is exactly the shape that lets one of them
        // be quietly wrong: a `Down` that is read as `Up` still moves a row,
        // still repaints, and still names the row it moved, so there is
        // nothing for a frontend -- or for a test that only checks that
        // *something* happened -- to notice. Only the resulting order can
        // tell the two apart, which is why this asserts on it and not on
        // `changed` alone.
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

        // And durable, the same as the other direction: an order that only
        // exists in the returned list springs back the next time anything
        // reloads the file.
        let reread = edit(&moved.text, Verb::Save { query: "" }).expect("the patched file parses");
        let keys: Vec<&str> = reread.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "c"], "the order did not survive the write");
    }

    #[test]
    fn a_row_at_the_end_it_is_moving_toward_stays_where_it_is() {
        // Both ends, over a list whose two ends are different rows. A
        // single-row fixture refuses every move whatever the code does with
        // the direction, so it proves the refusal and nothing about what the
        // refusal was for; here the front row may still go down and the back
        // row may still go up, which is the part that fails if a direction
        // is being dropped.
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
        // `rename_filter` answers "does this key name a search", which is not
        // the same question as "did anything change": it says yes to a rename
        // that stores the name already stored. Reporting that as a change
        // would rewrite `config.toml` for nothing -- the watcher wakes, the
        // sidebar repaints, and the keyboard is told to follow a row that
        // never moved -- so the comparison against the table as it was is
        // what settles it, and this is the only verb that can reach it.
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
            !gone.text.contains("[filters.urgent]"),
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
        // `pinned` is the whole of what "shows in the sidebar" means, and the
        // settings panel can turn it off without deleting the query.
        let text = "\
[filters.hidden]
query = \"is:unread\"
pinned = false
";
        let edit = edit(
            text,
            Verb::Save {
                query: "has:attach",
            },
        )
        .expect("the file parses");

        let keys: Vec<&str> = edit.searches.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["has-attach"]);
    }

    #[test]
    fn an_edit_leaves_every_other_line_in_the_file_alone() {
        // The reason this goes through `patch_filters` rather than
        // reserializing a `Config`: somebody's settings file is not ours to
        // reformat because they saved a search (#885).
        let saved = edit(
            SAMPLE,
            Verb::Save {
                query: "has:attach",
            },
        )
        .expect("the sample parses");

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
            saved.text.contains("[filters.has-attach]"),
            "and the edit itself still has to land:\n{}",
            saved.text
        );
    }

    #[test]
    fn a_file_that_will_not_parse_is_refused_rather_than_emptied() {
        // The dangerous reading is "a broken file tells us nothing, so start
        // from the defaults": the defaults have no filters, and the patch
        // would then write that emptiness over searches the user still has.
        let broken = "[ui]\ndensity = 42\n\n[filters.keep]\nquery = \"is:unread\"\n";

        assert!(
            edit(
                broken,
                Verb::Save {
                    query: "has:attach"
                }
            )
            .is_err(),
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

        // A machine that has never had a `config.toml` still gets its first
        // saved search.
        let saved = apply(&path, Verb::Save { query: "is:unread" }).expect("a first save");
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
        // Not a test of English. It is a test that the sentences exist here
        // at all: two platforms writing their own confirmation is two
        // products, and the destructive one is where that matters most.
        assert_eq!(DELETE_PROMPT.title, "Delete this saved search?");
        assert!(
            DELETE_PROMPT
                .body
                .is_some_and(|body| body.contains("query"))
        );
        assert_eq!(DELETE_PROMPT.confirm, "Delete");
        assert_eq!(RENAME_PROMPT.body, None);
    }
}
