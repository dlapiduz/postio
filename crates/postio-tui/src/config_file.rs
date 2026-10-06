//! What the terminal writes to `config.toml`, and reads back.
//!
//! The file is the settings, for both frontends (canvas 3f), and each of them
//! watches it: whatever the terminal writes here the desktop picks up the way
//! it picks up any edit (US7 scenario 3). So the terminal writes the way the
//! desktop does -- through `postio_config`'s `patch_*`, which touch one table
//! and leave the rest of the file, comments and all, as it was (#885).

use std::path::{Path, PathBuf};

use crate::places::{Features, Saved};

/// Where `config.toml` is.
pub fn path() -> Option<PathBuf> {
    postio_config::paths::config_path().ok()
}

/// The file's text, empty when there is none yet.
pub fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// What the terminal reads from `config.toml`: the pinned saved searches and
/// which of Focus's features are in use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    /// The pinned saved searches, in the finder's order.
    pub saved: Vec<Saved>,
    /// Which of Focus's features are in use.
    pub features: Features,
}

impl Read {
    /// What `config` says.
    pub fn of(config: &postio_config::Config) -> Read {
        Read {
            saved: pinned(config),
            features: Features {
                filtering: config.focus.filtering,
                digest_rules: config.focus.digests.len(),
                digests: crate::places::Rules(config.focus.digests.clone()),
                reading: config.focus.reading,
                capture: config.focus.vault.is_some(),
            },
        }
    }
}

/// Add `query` to `[filters]` at `path` as a pinned saved search, as the
/// desktop's Ctrl+S does, and answer the pinned searches now.
pub fn save_search(path: &Path, query: &str) -> Result<Read, String> {
    rewrite(path, |config| {
        config.save_filter(query);
    })
}

/// Change the filters in the file at `path` with `change`, leaving the rest
/// of it as it was.
fn rewrite(path: &Path, change: impl FnOnce(&mut postio_config::Config)) -> Result<Read, String> {
    let original = text(path);
    let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
    change(&mut config);
    let patched = postio_config::patch_filters(&original, &config.filters)
        .map_err(|error| error.to_string())?;
    postio_config::Config::write_text_to_path(&patched, path).map_err(|error| error.to_string())?;
    Ok(Read::of(&config))
}

/// Write `[focus] reading` at `path` as `reading`, leaving the rest of the
/// file as it was, as the desktop's `F8` does.
pub fn set_reading(path: &Path, reading: postio_config::Reading) -> Result<(), String> {
    let original = text(path);
    let written = postio_config::focus_edit::set_reading(&original, reading)
        .map_err(|error| error.to_string())?;
    match written {
        Some(edited) => postio_config::Config::write_text_to_path(&edited, path)
            .map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// The pinned saved searches in `config`, in the finder's order -- the
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

/// What the file at `path` says now.
pub fn read_at(path: &Path) -> Option<Read> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(Read::of(&postio_config::Config::from_toml_str(&text).ok()?))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn where_messages_open_is_written_in_focus_and_the_rest_of_the_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# my own notes\n[ui]\ntheme = \"dark\"\n").unwrap();
        set_reading(&path, postio_config::Reading::Pane).expect("written");
        let after = text(&path);
        assert!(
            after.contains("# my own notes") && after.contains("theme = \"dark\""),
            "{after}"
        );
        let config = postio_config::Config::from_toml_str(&after).unwrap();
        assert_eq!(config.focus.reading, postio_config::Reading::Pane);
        set_reading(&path, postio_config::Reading::Dialog).expect("written");
        let config = postio_config::Config::from_toml_str(&text(&path)).unwrap();
        assert_eq!(config.focus.reading, postio_config::Reading::Dialog);
    }

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
        assert_eq!(pinned.saved.len(), 1);
        assert_eq!(pinned.saved[0].query, "from:ada is:unread");

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
}
