//! Keeping a query: a `[saved_searches]` entry in `config.toml`, patched by
//! `postio_ui::saved_search` (Swift never parses TOML, ADR 0031).
//! The verbs take a path, not text: the file is hand-edited and watched, so a patch must read it at
//! the moment it acts.

use postio_ui::saved_search::{self, Verb};

use crate::settings::SettingsError;

/// `key` is the `[saved_searches.<key>]` identity, stable and TOML-safe so a rename cannot orphan
/// the entry (#292); `name` is what the user called it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedSearchFfi {
    /// The stable identity every verb below names. Never drawn.
    pub key: String,
    pub name: String,
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

/// The rows to draw now, always: a verb that did nothing still answers with the list as it stands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedSearchEditFfi {
    pub searches: Vec<SavedSearchFfi>,
    /// The key of the row that changed, or `None` when nothing did: after a reorder focus belongs
    /// on the row that moved.
    pub changed: Option<String>,
}

/// Best effort: a machine with no `config.toml`, or one that will not parse, shows no saved
/// searches rather than refusing to draw a sidebar.
#[uniffi::export]
pub fn saved_searches(path: String) -> Vec<SavedSearchFfi> {
    saved_search::load(std::path::Path::new(&path))
        .into_iter()
        .map(SavedSearchFfi::from)
        .collect()
}

/// The name is derived from the query text rather than asked for: the gesture is one keystroke, so
/// the row arrives called `is-unread-from-team`.
#[uniffi::export]
pub fn save_search(path: String, query: String) -> Result<SavedSearchEditFfi, SettingsError> {
    run(&path, Verb::save(&query))
}

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
