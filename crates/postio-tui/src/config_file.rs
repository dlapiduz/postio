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

/// Add `query` to `[filters]` at `path` as a pinned saved search, as the
/// desktop's Ctrl+S does, and answer the pinned searches now.
pub fn save_search(path: &Path, query: &str) -> Result<Vec<Saved>, String> {
    let original = text(path);
    let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
    config.save_filter(query);
    let patched = postio_config::patch_filters(&original, &config.filters)
        .map_err(|error| error.to_string())?;
    postio_config::Config::write_text_to_path(&patched, path).map_err(|error| error.to_string())?;
    Ok(pinned(&config))
}

/// The pinned saved searches in `config`, in the sidebar's order.
pub fn pinned(config: &postio_config::Config) -> Vec<Saved> {
    config
        .filters
        .iter()
        .filter(|(_, filter)| filter.pinned)
        .map(|(key, filter)| Saved {
            name: filter.name.clone().unwrap_or_else(|| key.clone()),
            query: filter.query.clone(),
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
}
