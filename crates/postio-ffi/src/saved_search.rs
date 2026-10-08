//! Keeping a query, as a frontend does it.
//!
//! A saved search is a `[saved_searches]` entry in `config.toml`, so **Swift never
//! parses or writes TOML** applies here exactly as it does to the settings
//! panes (ADR 0031): what crosses is a list of rows and the verb that adds one, and the
//! file is read, patched and written on this side by
//! [`postio_ui::saved_search`] — the same code the classic app ran, so a search
//! saved on a Mac and one saved on Linux are the same edit.
//!
//! # Why these take a path rather than the file's text
//!
//! The settings surface passes text both ways because the pane *shows* the
//! file and can hold it. A sidebar holds no such thing, and a verb invoked
//! from it has to read the file at the moment it acts: `config.toml` is
//! hand-edited and watched, and patching a copy read when the window opened
//! would write an hour-old `[sync]` block back over a newer one. The path
//! comes from [`crate::settings_path`], which is the one answer to where the
//! file lives on this platform.
//!
//! # What is not here
//!
//! Running a saved search, and renaming, reordering or deleting one: the
//! sidebar that did those went with the three-pane app, and Focus runs a
//! saved search from the command bar (specs/009-focus-macos T082).

use postio_ui::saved_search::{self, Verb};

use crate::settings::SettingsError;

/// One saved search, as a sidebar row.
///
/// The `key` is the `[saved_searches.<key>]` identity and is **not** a label: #292
/// keeps it stable and TOML-safe so a rename cannot orphan the entry, and
/// `name` is whatever the user actually called it — the key itself, until
/// they call it something.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedSearchFfi {
    /// The stable `[saved_searches.<key>]` identity, and what every verb below
    /// names. Never drawn.
    pub key: String,
    /// What the row shows.
    pub name: String,
    /// The query this row runs when it is picked.
    pub query: String,
}

impl From<saved_search::SavedSearch> for SavedSearchFfi {
    fn from(search: saved_search::SavedSearch) -> Self {
        Self {
            key: search.key,
            name: search.name,
            query: search.query,
        }
    }
}

/// What a saved-search verb left behind.
///
/// The rows to draw now, always — a verb that did nothing still answers with
/// the list as it stands, so a frontend has one thing to do with the result
/// rather than two.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedSearchEditFfi {
    /// Every pinned saved search, in the order the sidebar draws them.
    pub searches: Vec<SavedSearchFfi>,
    /// The key of the row that changed, or `None` when nothing did — a blank
    /// query, a key naming no search, a row already at the end it was moving
    /// toward.
    ///
    /// Load-bearing for the keyboard: after a reorder the focus belongs on
    /// the row that moved, not on the position it vacated, and `None` is how
    /// a frontend knows not to flash a change that did not happen.
    pub changed: Option<String>,
}

/// Every saved search in the file at `path`, in sidebar order.
///
/// Best effort, like [`crate::settings_load`]: a machine with no
/// `config.toml` yet has no saved searches, and one whose file will not parse
/// shows none rather than refusing to draw a sidebar. The errors worth
/// raising are the ones the verbs below return, where a write was about to
/// happen.
#[uniffi::export]
pub fn saved_searches(path: String) -> Vec<SavedSearchFfi> {
    saved_search::load(std::path::Path::new(&path))
        .into_iter()
        .map(SavedSearchFfi::from)
        .collect()
}

/// Keep `query` as a new saved search, pinned to the sidebar.
///
/// `save_search`'s write path. The name is derived from the query text
/// rather than asked for — the gesture is one keystroke, and a dialog in
/// front of it is what stops people saving searches at all — so the row
/// arrives called `is-unread-from-team` and is renamed if that bothers
/// anyone.
#[uniffi::export]
pub fn save_search(path: String, query: String) -> Result<SavedSearchEditFfi, SettingsError> {
    run(&path, Verb::Save { query: &query })
}

/// Run `verb` against the file at `path` and shape the answer for a frontend.
///
/// One place, so the exported verbs above are each a verb and nothing else.
fn run(path: &str, verb: Verb<'_>) -> Result<SavedSearchEditFfi, SettingsError> {
    saved_search::apply(std::path::Path::new(path), verb)
        .map(|edit| SavedSearchEditFfi {
            searches: edit
                .searches
                .into_iter()
                .map(SavedSearchFfi::from)
                .collect(),
            changed: edit.changed,
        })
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })
}
