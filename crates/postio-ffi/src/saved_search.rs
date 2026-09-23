//! Keeping a query, as a frontend does it.
//!
//! A saved search is a `[filters]` entry in `config.toml`, so **Swift never
//! parses or writes TOML** applies here exactly as it does to the settings
//! panes (ADR 0031): what crosses is a list of rows and four verbs, and the
//! file is read, patched and written on this side by
//! [`postio_ui::saved_search`] — the same code `postio-gtk` runs, so a search
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
//! Running a saved search. Picking a row hands its
//! [`SavedSearchFfi::query`] to `Session::search`, which is the same
//! function the search field calls — a saved search is a query that was
//! written down, not a second kind of thing to open.

use postio_ui::saved_search::{self, Verb};

use crate::settings::SettingsError;

/// One saved search, as a sidebar row.
///
/// The `key` is the `[filters.<key>]` identity and is **not** a label: #292
/// keeps it stable and TOML-safe so a rename cannot orphan the entry, and
/// `name` is whatever the user actually called it — the key itself, until
/// they call it something.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedSearchFfi {
    /// The stable `[filters.<key>]` identity, and what every verb below
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

/// Which way [`move_saved_search`] walks a row.
///
/// One function and a direction rather than two functions, because the two
/// command ids differ by exactly this and nothing else — a second copy of the
/// body is a second place for the reorder rule to drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReorderFfi {
    /// Toward the front of the list.
    Up,
    /// Toward the back.
    Down,
}

impl From<ReorderFfi> for saved_search::Reorder {
    fn from(direction: ReorderFfi) -> Self {
        match direction {
            ReorderFfi::Up => saved_search::Reorder::Up,
            ReorderFfi::Down => saved_search::Reorder::Down,
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

/// The words a confirmation asks in.
///
/// Wording crosses because wording drifts (ADR 0019 Q6), and the two
/// platforms writing their own sentence for a destructive verb is two
/// products. Saved searches are the first two questions to cross; the record
/// is shaped for the next one rather than for these two.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PromptFfi {
    /// The question.
    pub title: String,
    /// What it costs, when the title does not already say.
    pub body: Option<String>,
    /// The button that goes through with it.
    pub confirm: String,
    /// The button that does not.
    pub cancel: String,
}

impl From<saved_search::Prompt> for PromptFfi {
    fn from(prompt: saved_search::Prompt) -> Self {
        Self {
            title: prompt.title.to_owned(),
            body: prompt.body.map(str::to_owned),
            confirm: prompt.confirm.to_owned(),
            cancel: prompt.cancel.to_owned(),
        }
    }
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

/// Give the saved search under `key` a display name of its own.
///
/// Blank, or the key typed back, clears the name rather than storing one:
/// the row falls back to drawing its key, which is what it did before anyone
/// renamed it.
#[uniffi::export]
pub fn rename_saved_search(
    path: String,
    key: String,
    name: String,
) -> Result<SavedSearchEditFfi, SettingsError> {
    run(
        &path,
        Verb::Rename {
            key: &key,
            name: &name,
        },
    )
}

/// Move the saved search under `key` one place.
///
/// Nothing to confirm and nothing to undo: moving it back is the same verb
/// once more, which is why `CommandId::MoveSavedSearchUp` declares
/// `Recovery::None`.
#[uniffi::export]
pub fn move_saved_search(
    path: String,
    key: String,
    direction: ReorderFfi,
) -> Result<SavedSearchEditFfi, SettingsError> {
    run(
        &path,
        Verb::Move {
            key: &key,
            direction: direction.into(),
        },
    )
}

/// Remove the saved search under `key`.
///
/// Ask first — [`saved_search_delete_prompt`] has the words. A config-file
/// edit has no undo stack to reach, so `CommandId::DeleteSavedSearch`
/// declares `Recovery::Confirm` and a frontend that deleted without asking
/// would be breaking the invariant `PRODUCT.md` states: destructive
/// operations are confirmed or undoable, and this one cannot be the second.
#[uniffi::export]
pub fn delete_saved_search(path: String, key: String) -> Result<SavedSearchEditFfi, SettingsError> {
    run(&path, Verb::Delete { key: &key })
}

/// What to ask before deleting a saved search.
#[uniffi::export]
pub fn saved_search_delete_prompt() -> PromptFfi {
    saved_search::DELETE_PROMPT.into()
}

/// What to ask when renaming one. The entry is pre-filled with
/// [`SavedSearchFfi::name`], which the sidebar already holds.
#[uniffi::export]
pub fn saved_search_rename_prompt() -> PromptFfi {
    saved_search::RENAME_PROMPT.into()
}

/// Run `verb` against the file at `path` and shape the answer for a frontend.
///
/// One place, so the five exported functions above are each a verb and
/// nothing else.
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
