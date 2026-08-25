//! Where Postio keeps its local store.
//!
//! `postio-config` owns the *configuration* path and deliberately says nothing
//! about the database: the store is not a setting, and the crate that reads
//! `config.toml` has no business knowing where SQLite lives. Choosing that is
//! the composition root's job, which is here.

use std::path::PathBuf;

/// Overrides everything when set. For a second profile, and for tests.
const STORE_PATH_ENV: &str = "POSTIO_STORE";

/// Overrides where dragged-out mail is written. For tests.
const EXPORT_PATH_ENV: &str = "POSTIO_EXPORT_DIR";

/// The database file.
///
/// `$XDG_DATA_HOME/postio/postio.db`, falling back to
/// `$HOME/.local/share/postio/postio.db`. Data rather than config or cache:
/// this is the user's mail, so it is neither a preference they can retype nor
/// something safe to delete.
pub fn store_path() -> PathBuf {
    store_path_from(|key| std::env::var(key).ok())
}

/// As [`store_path`], for an arbitrary environment lookup.
fn store_path_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(STORE_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    let directory = env("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        // Nowhere to put it: the working directory is a poor answer and a
        // loud one, which is better than a silent one somewhere surprising.
        .unwrap_or_else(|| PathBuf::from("."));
    directory.join("postio").join("postio.db")
}

/// Where messages dragged out of Postio are written.
///
/// `$XDG_CACHE_HOME/postio/drag`. **Cache, not data**, and the distinction is
/// the whole reason this is a separate function rather than a folder beside
/// the database: these files are copies of mail that is already stored, made
/// so that some other application could be handed a path. A cache directory
/// is somewhere the system is allowed to reclaim, which is the right shape
/// for files whose only purpose was to survive long enough for a drop to
/// read them.
///
/// # "Losing them costs nothing" is not quite true
///
/// It is true of the *bytes* — they are still in the blob store — and false
/// of the timing. A receiver is handed a **path**, and it reads that path
/// after the drop, on its own schedule. That is so even through the file
/// transfer portal, which despite taking file descriptors hands the receiver
/// back paths and never copies the content (#121, and
/// `crates/postio-app/tests/drag_out_portal.rs`). So a file reclaimed between
/// the drop and the read is a drop that silently produced nothing, and
/// nothing anywhere reports an error.
///
/// Nothing in Postio deletes this directory today, which is why that window
/// never opens in practice. **Do not add a sweep over it without reading that
/// test first** — a startup purge here would be the same class of bug as
/// pointing this at the blob store's temporary directory, below.
///
/// Not the blob store's temporary directory, which looks tempting and is
/// wrong: `BlobStore::purge_temporary` deletes everything in there on start,
/// and it exists for half-finished writes rather than for files another
/// process is about to open.
pub fn export_dir() -> PathBuf {
    export_dir_from(|key| std::env::var(key).ok())
}

/// As [`export_dir`], for an arbitrary environment lookup.
fn export_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(EXPORT_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    let directory = env("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    directory.join("postio").join("drag")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| {
            pairs
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn the_store_lives_under_the_data_directory() {
        assert_eq!(
            store_path_from(env(&[("XDG_DATA_HOME", "/data")])),
            PathBuf::from("/data/postio/postio.db")
        );
        assert_eq!(
            store_path_from(env(&[("HOME", "/home/ada")])),
            PathBuf::from("/home/ada/.local/share/postio/postio.db"),
            "not $HOME/.config: mail is data, not a preference"
        );
    }

    #[test]
    fn an_explicit_path_wins() {
        assert_eq!(
            store_path_from(env(&[
                ("POSTIO_STORE", "/tmp/other.db"),
                ("XDG_DATA_HOME", "/data"),
            ])),
            PathBuf::from("/tmp/other.db")
        );
    }

    #[test]
    fn dragged_out_mail_is_cache_not_data() {
        // A copy of mail that is already stored. The system may reclaim it;
        // the mail it came from it may not.
        assert_eq!(
            export_dir_from(env(&[
                ("XDG_CACHE_HOME", "/cache"),
                ("XDG_DATA_HOME", "/data"),
            ])),
            PathBuf::from("/cache/postio/drag"),
            "exports must not land beside the database"
        );
        assert_eq!(
            export_dir_from(env(&[("HOME", "/home/ada")])),
            PathBuf::from("/home/ada/.cache/postio/drag")
        );
    }

    #[test]
    fn the_export_directory_can_be_pointed_somewhere_else() {
        assert_eq!(
            export_dir_from(env(&[
                ("POSTIO_EXPORT_DIR", "/tmp/drag"),
                ("XDG_CACHE_HOME", "/cache"),
            ])),
            PathBuf::from("/tmp/drag")
        );
    }

    #[test]
    fn an_empty_override_is_not_an_override() {
        assert_eq!(
            store_path_from(env(&[("POSTIO_STORE", ""), ("XDG_DATA_HOME", "/data")])),
            PathBuf::from("/data/postio/postio.db")
        );
    }
}
