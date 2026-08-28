//! Where Postio keeps its local store.
//!
//! `postio-config` owns the *configuration* path and deliberately says nothing
//! about the database: the store is not a setting, and the crate that reads
//! `config.toml` has no business knowing where SQLite lives. Choosing that is
//! the composition root's job, which is here.

use std::path::PathBuf;

pub use postio_config::paths::Platform;

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
    store_path_from(|key| std::env::var(key).ok(), Platform::host())
}

/// As [`store_path`], for an arbitrary environment lookup and layout.
///
/// On [`Platform::Apple`] the answer is
/// `~/Library/Application Support/Postio/postio.db` — the platform's own
/// convention, which is where a Mac user looks and where every backup tool
/// already knows to go (#556).
///
/// `$POSTIO_STORE` and a deliberate `$XDG_DATA_HOME` still win on either
/// platform. Someone who set one meant it, and it is what lets a store be
/// shared with a Linux VM on the same machine — and what keeps every fixture
/// and `scripts/run-isolated.sh` working untouched.
fn store_path_from<F>(env: F, platform: Platform) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(STORE_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    if let Some(xdg) = env("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(xdg).join("postio").join("postio.db");
    }
    let Some(home) = env("HOME").filter(|value| !value.is_empty()) else {
        // Nowhere to put it: the working directory is a poor answer and a
        // loud one, which is better than a silent one somewhere surprising.
        return PathBuf::from(".").join("postio").join("postio.db");
    };
    match platform {
        Platform::Apple => Platform::apple_support_dir(&home).join("postio.db"),
        Platform::Freedesktop => PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("postio")
            .join("postio.db"),
    }
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
/// This directory *is* swept, at startup, by
/// [`reclaim_drag_exports`](crate::reclaim_drag_exports) — it used to grow
/// without bound, which was a privacy problem rather than only a disk one
/// (#278). What keeps that sweep from being the bug described above is that
/// it deletes nothing younger than
/// [`DRAG_EXPORT_GRACE_PERIOD`](crate::DRAG_EXPORT_GRACE_PERIOD): a file a
/// drop is still using is seconds old.
///
/// **Do not replace that age guard with a plain purge**, and read
/// `crates/postio-app/tests/app_suite/drag_out_portal.rs` before changing it.
/// A purge here would be the same class of bug as pointing this at the blob
/// store's temporary directory, below.
///
/// Not the blob store's temporary directory, which looks tempting and is
/// wrong: `BlobStore::purge_temporary` deletes everything in there on start,
/// and it exists for half-finished writes rather than for files another
/// process is about to open.
pub fn export_dir() -> PathBuf {
    export_dir_from(|key| std::env::var(key).ok(), Platform::host())
}

/// As [`export_dir`], for an arbitrary environment lookup and layout.
///
/// On [`Platform::Apple`], `~/Library/Caches/Postio/drag`. Still *cache*
/// rather than data on either platform, for the reason above: these are
/// copies of mail that is already stored, and Apple's reclaimable directory
/// is `~/Library/Caches`, not `Application Support`.
fn export_dir_from<F>(env: F, platform: Platform) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(EXPORT_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    if let Some(xdg) = env("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(xdg).join("postio").join("drag");
    }
    let Some(home) = env("HOME").filter(|value| !value.is_empty()) else {
        return PathBuf::from(".").join("postio").join("drag");
    };
    match platform {
        Platform::Apple => PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("Postio")
            .join("drag"),
        Platform::Freedesktop => PathBuf::from(home)
            .join(".cache")
            .join("postio")
            .join("drag"),
    }
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
    fn on_apple_the_store_lives_where_apple_puts_it() {
        // Decided by the maintainer, 2026-08-27 (#556): the platform's own
        // convention, not the XDG layout wearing a mac hat. This is where a
        // Mac user looks and where every backup tool already knows to go.
        assert_eq!(
            store_path_from(env(&[("HOME", "/Users/ada")]), Platform::Apple),
            PathBuf::from("/Users/ada/Library/Application Support/Postio/postio.db")
        );
    }

    #[test]
    fn both_platforms_answer_from_either_host() {
        // Why this is a parameter and not a `#[cfg]`. A `cfg` would mean each
        // machine could only ever prove half of this, so the half nobody runs
        // is the half that rots -- and the macOS answer is exactly the one
        // most sessions cannot check.
        let home = env(&[("HOME", "/home/ada")]);
        assert_eq!(
            store_path_from(&home, Platform::Freedesktop),
            PathBuf::from("/home/ada/.local/share/postio/postio.db")
        );
        assert_eq!(
            store_path_from(&home, Platform::Apple),
            PathBuf::from("/home/ada/Library/Application Support/Postio/postio.db")
        );
    }

    #[test]
    fn an_explicit_path_still_wins_on_apple() {
        // Every fixture, `scripts/run-isolated.sh` and the tests that set
        // these directly depend on this staying true on both platforms.
        assert_eq!(
            store_path_from(
                env(&[("POSTIO_STORE", "/tmp/other.db"), ("HOME", "/Users/ada")]),
                Platform::Apple
            ),
            PathBuf::from("/tmp/other.db")
        );
    }

    #[test]
    fn an_explicit_xdg_home_still_wins_on_apple() {
        // A deliberate `$XDG_DATA_HOME` is a person saying where they want it,
        // and the platform default has no business overruling that. It is also
        // what lets one store be shared with a Linux VM on the same machine.
        assert_eq!(
            store_path_from(
                env(&[("XDG_DATA_HOME", "/data"), ("HOME", "/Users/ada")]),
                Platform::Apple
            ),
            PathBuf::from("/data/postio/postio.db")
        );
    }

    #[test]
    fn on_apple_dragged_out_mail_goes_to_library_caches() {
        // Still cache rather than data, for the reason the freedesktop case
        // gives: these are copies of mail that is already stored. Apple's
        // reclaimable directory is `~/Library/Caches`.
        assert_eq!(
            export_dir_from(env(&[("HOME", "/Users/ada")]), Platform::Apple),
            PathBuf::from("/Users/ada/Library/Caches/Postio/drag")
        );
    }

    #[test]
    fn the_store_lives_under_the_data_directory() {
        assert_eq!(
            store_path_from(env(&[("XDG_DATA_HOME", "/data")]), Platform::Freedesktop),
            PathBuf::from("/data/postio/postio.db")
        );
        assert_eq!(
            store_path_from(env(&[("HOME", "/home/ada")]), Platform::Freedesktop),
            PathBuf::from("/home/ada/.local/share/postio/postio.db"),
            "not $HOME/.config: mail is data, not a preference"
        );
    }

    #[test]
    fn an_explicit_path_wins() {
        assert_eq!(
            store_path_from(
                env(&[
                    ("POSTIO_STORE", "/tmp/other.db"),
                    ("XDG_DATA_HOME", "/data")
                ]),
                Platform::Freedesktop
            ),
            PathBuf::from("/tmp/other.db")
        );
    }

    #[test]
    fn dragged_out_mail_is_cache_not_data() {
        // A copy of mail that is already stored. The system may reclaim it;
        // the mail it came from it may not.
        assert_eq!(
            export_dir_from(
                env(&[("XDG_CACHE_HOME", "/cache"), ("XDG_DATA_HOME", "/data")]),
                Platform::Freedesktop
            ),
            PathBuf::from("/cache/postio/drag"),
            "exports must not land beside the database"
        );
        assert_eq!(
            export_dir_from(env(&[("HOME", "/home/ada")]), Platform::Freedesktop),
            PathBuf::from("/home/ada/.cache/postio/drag")
        );
    }

    #[test]
    fn the_export_directory_can_be_pointed_somewhere_else() {
        assert_eq!(
            export_dir_from(
                env(&[
                    ("POSTIO_EXPORT_DIR", "/tmp/drag"),
                    ("XDG_CACHE_HOME", "/cache"),
                ]),
                Platform::Freedesktop
            ),
            PathBuf::from("/tmp/drag")
        );
    }

    #[test]
    fn an_empty_override_is_not_an_override() {
        assert_eq!(
            store_path_from(
                env(&[("POSTIO_STORE", ""), ("XDG_DATA_HOME", "/data")]),
                Platform::Freedesktop
            ),
            PathBuf::from("/data/postio/postio.db")
        );
    }
}
