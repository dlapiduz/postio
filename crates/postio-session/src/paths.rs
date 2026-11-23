//! Where Postio keeps its local store; the composition root chooses it, since `postio-config` owns
//! only the configuration path.

use std::path::PathBuf;

pub use postio_config::paths::Platform;

const STORE_PATH_ENV: &str = "POSTIO_STORE";

const EXPORT_PATH_ENV: &str = "POSTIO_EXPORT_DIR";

/// The database file: `$XDG_DATA_HOME/postio/postio.db`, else `$HOME/.local/share/...`. Data, not
/// config or cache: it is the user's mail.
pub fn store_path() -> PathBuf {
    store_path_from(|key| std::env::var(key).ok(), Platform::host())
}

/// As `store_path`, for an arbitrary environment lookup and layout. On `Platform::Apple` it is
/// `~/Library/Application Support/Postio/postio.db` (#556); `$POSTIO_STORE` and a deliberate
/// `$XDG_DATA_HOME` still win.
fn store_path_from<F>(env: F, platform: Platform) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(STORE_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    if let Some(xdg) = postio_config::paths::shared_xdg(&env, "XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("postio").join("postio.db");
    }
    let Some(home) = env("HOME").filter(|value| !value.is_empty()) else {
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

/// Where messages dragged out of Postio are written: `$XDG_CACHE_HOME/postio/drag`. Cache, not
/// data: these are copies of stored mail.
///
/// A receiver reads the path after the drop, on its own schedule, even through the file transfer
/// portal (#121), so a file reclaimed too early is a silent empty drop. The startup sweep
/// (`reclaim_drag_exports`) keeps an age guard; do not replace it with a plain purge, and do not
/// point this at the blob store's temporary directory, which `purge_temporary` empties on start.
pub fn export_dir() -> PathBuf {
    export_dir_from(|key| std::env::var(key).ok(), Platform::host())
}

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
        assert_eq!(
            store_path_from(env(&[("HOME", "/Users/ada")]), Platform::Apple),
            PathBuf::from("/Users/ada/Library/Application Support/Postio/postio.db")
        );
    }

    #[test]
    fn both_platforms_answer_from_either_host() {
        // A parameter, not a `#[cfg]`: with `cfg` each machine proves only half, and the half
        // nobody runs rots.
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
    fn inside_a_flatpak_the_store_is_the_hosts() {
        let sandboxed = [
            ("FLATPAK_ID", "dev.postio.Postio"),
            ("XDG_DATA_HOME", "/home/ada/.var/app/dev.postio.Postio/data"),
            ("HOME", "/home/ada"),
        ];
        assert_eq!(
            store_path_from(env(&sandboxed), Platform::Freedesktop),
            PathBuf::from("/home/ada/.local/share/postio/postio.db")
        );
        let mut with_host = sandboxed.to_vec();
        with_host.push(("HOST_XDG_DATA_HOME", "/data"));
        assert_eq!(
            store_path_from(env(&with_host), Platform::Freedesktop),
            PathBuf::from("/data/postio/postio.db")
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
