//! Where `config.toml` lives.
//!
//! `~/.config/postio/config.toml` is the single source of truth: the settings
//! panel edits this same file, and there is no separate settings store.

use std::path::PathBuf;

use crate::error::{ConfigError, Result};

/// Which platform's directory layout to answer with.
///
/// A parameter rather than a `#[cfg]`, and that is the point: with a `cfg`
/// each machine could only ever prove half of this, and the half nobody runs
/// is the half that rots — which would be the macOS answer, the one most
/// sessions cannot check. As a parameter both layouts are asserted from either
/// host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// The XDG base directory specification: Linux, the BSDs.
    Freedesktop,
    /// Apple's layout: `~/Library/Application Support`, `~/Library/Caches`.
    Apple,
}

impl Platform {
    /// What this build is running on.
    pub const fn host() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Apple
        } else {
            Platform::Freedesktop
        }
    }

    /// The directory Apple keeps an application's own files in.
    ///
    /// `Postio`, capitalised, because that is the convention on the platform
    /// and this directory is shown to people — Finder lists it, and every
    /// backup tool already knows to include it.
    pub fn apple_support_dir(home: &str) -> PathBuf {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Postio")
    }
}

/// File name inside the config directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Environment variable that overrides the whole path, used by tests and by
/// `postio --config`.
pub const CONFIG_PATH_ENV: &str = "POSTIO_CONFIG";

/// The directory holding Postio's configuration, resolved from an arbitrary
/// environment lookup so it is testable without touching the process
/// environment.
///
/// `$XDG_CONFIG_HOME/postio`, falling back to `$HOME/.config/postio` — or, on
/// [`Platform::Apple`], `~/Library/Application Support/Postio`.
///
/// **`$XDG_CONFIG_HOME` still wins on Apple.** Someone who set it meant it,
/// and the platform default has no business overruling a deliberate choice;
/// it is also what lets one configuration be shared with a Linux VM on the
/// same machine, and what keeps every fixture working unchanged.
pub fn config_dir_from<F>(env: F, platform: Platform) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(xdg) = env("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("postio"));
    }
    if let Some(home) = env("HOME").filter(|v| !v.is_empty()) {
        return Ok(match platform {
            Platform::Apple => Platform::apple_support_dir(&home),
            Platform::Freedesktop => PathBuf::from(home).join(".config").join("postio"),
        });
    }
    Err(ConfigError::NoConfigDir)
}

/// The directory holding Postio's configuration.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_from(|key| std::env::var(key).ok(), Platform::host())
}

/// Full path to `config.toml`, resolved from an arbitrary environment lookup.
///
/// `$POSTIO_CONFIG` overrides everything when set.
pub fn config_path_from<F>(env: F, platform: Platform) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(CONFIG_PATH_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    Ok(config_dir_from(env, platform)?.join(CONFIG_FILE_NAME))
}

/// Full path to `config.toml`.
pub fn config_path() -> Result<PathBuf> {
    config_path_from(|key| std::env::var(key).ok(), Platform::host())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn xdg_config_home_wins_over_home() {
        let dir = config_dir_from(
            env_of(&[("XDG_CONFIG_HOME", "/x/conf"), ("HOME", "/home/p")]),
            Platform::Freedesktop,
        )
        .unwrap();
        assert_eq!(dir, PathBuf::from("/x/conf/postio"));
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let dir = config_dir_from(
            env_of(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/p")]),
            Platform::Freedesktop,
        )
        .unwrap();
        assert_eq!(dir, PathBuf::from("/home/p/.config/postio"));
    }

    #[test]
    fn the_explicit_override_wins() {
        let path = config_path_from(
            env_of(&[(CONFIG_PATH_ENV, "/tmp/alt.toml"), ("HOME", "/home/p")]),
            Platform::Freedesktop,
        )
        .unwrap();
        assert_eq!(path, PathBuf::from("/tmp/alt.toml"));
    }

    #[test]
    fn the_default_path_is_dot_config_postio() {
        let path = config_path_from(env_of(&[("HOME", "/home/p")]), Platform::Freedesktop).unwrap();
        assert_eq!(path, PathBuf::from("/home/p/.config/postio/config.toml"));
    }

    #[test]
    fn on_apple_config_lives_in_application_support() {
        let path = config_path_from(env_of(&[("HOME", "/Users/ada")]), Platform::Apple).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/Users/ada/Library/Application Support/Postio/config.toml")
        );
    }

    #[test]
    fn an_explicit_xdg_config_home_still_wins_on_apple() {
        let dir = config_dir_from(
            env_of(&[("XDG_CONFIG_HOME", "/x/conf"), ("HOME", "/Users/ada")]),
            Platform::Apple,
        )
        .unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/x/conf/postio"),
            "a deliberate XDG_CONFIG_HOME is a person saying where they want it"
        );
    }

    #[test]
    fn both_layouts_answer_from_either_host() {
        let home = env_of(&[("HOME", "/home/ada")]);
        assert_eq!(
            config_dir_from(&home, Platform::Freedesktop).unwrap(),
            PathBuf::from("/home/ada/.config/postio")
        );
        assert_eq!(
            config_dir_from(&home, Platform::Apple).unwrap(),
            PathBuf::from("/home/ada/Library/Application Support/Postio")
        );
    }

    #[test]
    fn without_any_environment_there_is_no_path() {
        assert!(matches!(
            config_dir_from(|_| None, Platform::Freedesktop),
            Err(ConfigError::NoConfigDir)
        ));
    }
}
