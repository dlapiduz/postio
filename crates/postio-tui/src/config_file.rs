//! What the terminal writes to `config.toml`, and reads back.
//!
//! The file is the settings, for both frontends (canvas 3f), and each of them
//! watches it: whatever the terminal writes here the desktop picks up the way
//! it picks up any edit (US7 scenario 3). So the terminal writes the way the
//! desktop does -- through `postio_config`'s `patch_*`, which touch one table
//! and leave the rest of the file, comments and all, as it was (#885).

use std::path::{Path, PathBuf};

use crate::sidebar::Saved;

/// Where `config.toml` is.
pub fn path() -> Option<PathBuf> {
    postio_config::paths::config_path().ok()
}

/// The file's text, empty when there is none yet.
pub fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A change to a saved search, as the desktop's sidebar makes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEdit {
    /// Call it `name`; empty goes back to its key.
    Rename {
        /// Its `[filters]` key.
        key: String,
        /// What to call it.
        name: String,
    },
    /// One place earlier (`up`) or later in the sidebar.
    Move {
        /// Its `[filters]` key.
        key: String,
        /// Toward the top.
        up: bool,
    },
    /// Take it out of the file.
    Delete {
        /// Its `[filters]` key.
        key: String,
    },
}

/// Add `query` to `[filters]` at `path` as a pinned saved search, as the
/// desktop's Ctrl+S does, and answer the pinned searches now.
pub fn save_search(path: &Path, query: &str) -> Result<Vec<Saved>, String> {
    rewrite(path, |config| {
        config.save_filter(query);
    })
}

/// Make `edit` to `[filters]` at `path`, through the same `postio_config`
/// calls the desktop's sidebar makes, and answer the pinned searches now.
pub fn edit_search(path: &Path, edit: &SearchEdit) -> Result<Vec<Saved>, String> {
    use postio_config::filters::Reorder;
    rewrite(path, |config| {
        match edit {
            SearchEdit::Rename { key, name } => config.rename_filter(key, name),
            SearchEdit::Move { key, up } => {
                config.move_filter(key, if *up { Reorder::Up } else { Reorder::Down })
            }
            SearchEdit::Delete { key } => config.delete_filter(key),
        };
    })
}

/// Change the filters in the file at `path` with `change`, leaving the rest
/// of it as it was.
fn rewrite(
    path: &Path,
    change: impl FnOnce(&mut postio_config::Config),
) -> Result<Vec<Saved>, String> {
    let original = text(path);
    let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
    change(&mut config);
    let patched = postio_config::patch_filters(&original, &config.filters)
        .map_err(|error| error.to_string())?;
    postio_config::Config::write_text_to_path(&patched, path).map_err(|error| error.to_string())?;
    Ok(pinned(&config))
}

/// The pinned saved searches in `config`, in the sidebar's order -- the
/// one a reorder on either app writes.
pub fn pinned(config: &postio_config::Config) -> Vec<Saved> {
    config
        .ordered_filter_keys()
        .into_iter()
        .filter_map(|key| {
            let filter = config.filters.get(&key)?;
            Some(Saved {
                name: filter.name.clone().unwrap_or_else(|| key.clone()),
                query: filter.query.clone(),
                key,
            })
        })
        .collect()
}

/// The pinned saved searches as the file at `path` says now.
pub fn pinned_at(path: &Path) -> Option<Vec<Saved>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(pinned(&postio_config::Config::from_toml_str(&text).ok()?))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn what_the_terminal_writes_the_desktops_watcher_sees() {
        // US7 scenario 3 / T088: the desktop reloads config.toml through
        // this same watcher, so a change the terminal makes reaches it live.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# my own notes\n[ui]\ntheme = \"dark\"\n").unwrap();
        let (seen, heard) = mpsc::channel();
        let _watching = postio_config::watch::ConfigWatcher::new(&path, move |checked| {
            let _ = seen.send(checked);
        })
        .expect("watching");

        let pinned = save_search(&path, "from:ada is:unread").expect("saved");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].query, "from:ada is:unread");

        let checked = heard
            .recv_timeout(Duration::from_secs(10))
            .expect("the watcher heard the write");
        let config = checked.config.expect("the file is still valid");
        assert!(
            config
                .filters
                .values()
                .any(|filter| filter.query == "from:ada is:unread")
        );
        assert!(
            text(&path).contains("# my own notes"),
            "the rest of the file is left as it was"
        );
    }

    #[test]
    fn a_saved_search_is_renamed_moved_and_deleted_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# my own notes\n").unwrap();
        save_search(&path, "from:ada").unwrap();
        let pinned = save_search(&path, "is:unread").unwrap();
        let keys: Vec<String> = pinned.iter().map(|saved| saved.key.clone()).collect();
        assert_eq!(keys.len(), 2);

        let pinned = edit_search(
            &path,
            &SearchEdit::Rename {
                key: keys[0].clone(),
                name: "Ada".into(),
            },
        )
        .unwrap();
        assert_eq!(pinned[0].name, "Ada");
        assert_eq!(pinned[0].query, "from:ada");

        // The order is the file's, which the desktop reads too.
        let pinned = edit_search(
            &path,
            &SearchEdit::Move {
                key: keys[0].clone(),
                up: false,
            },
        )
        .unwrap();
        let order: Vec<&str> = pinned.iter().map(|saved| saved.query.as_str()).collect();
        assert_eq!(order, ["is:unread", "from:ada"]);
        assert_eq!(pinned_at(&path).unwrap(), pinned, "as the file says");

        let pinned = edit_search(
            &path,
            &SearchEdit::Delete {
                key: keys[1].clone(),
            },
        )
        .unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].name, "Ada");
        assert!(text(&path).contains("# my own notes"));
    }
}
