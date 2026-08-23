//! Where `config.toml` lives.
//!
//! `~/.config/postio/config.toml` is the single source of truth: the settings
//! panel edits this same file, and there is no separate settings store.

use std::path::PathBuf;

use crate::error::{ConfigError, Result};

/// File name inside the config directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Environment variable that overrides the whole path, used by tests and by
/// `postio --config`.
pub const CONFIG_PATH_ENV: &str = "POSTIO_CONFIG";

/// The directory holding Postio's configuration, resolved from an arbitrary
/// environment lookup so it is testable without touching the process
/// environment.
///
/// `$XDG_CONFIG_HOME/postio`, falling back to `$HOME/.config/postio`.
pub fn config_dir_from<F>(env: F) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(xdg) = env("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("postio"));
    }
    if let Some(home) = env("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home).join(".config").join("postio"));
    }
    Err(ConfigError::NoConfigDir)
}

/// The directory holding Postio's configuration.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_from(|key| std::env::var(key).ok())
}

/// Full path to `config.toml`, resolved from an arbitrary environment lookup.
///
/// `$POSTIO_CONFIG` overrides everything when set.
pub fn config_path_from<F>(env: F) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(CONFIG_PATH_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    Ok(config_dir_from(env)?.join(CONFIG_FILE_NAME))
}

/// Full path to `config.toml`.
pub fn config_path() -> Result<PathBuf> {
    config_path_from(|key| std::env::var(key).ok())
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
        let dir = config_dir_from(env_of(&[
            ("XDG_CONFIG_HOME", "/x/conf"),
            ("HOME", "/home/p"),
        ]))
        .unwrap();
        assert_eq!(dir, PathBuf::from("/x/conf/postio"));
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let dir = config_dir_from(env_of(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/p")])).unwrap();
        assert_eq!(dir, PathBuf::from("/home/p/.config/postio"));
    }

    #[test]
    fn the_explicit_override_wins() {
        let path = config_path_from(env_of(&[
            (CONFIG_PATH_ENV, "/tmp/alt.toml"),
            ("HOME", "/home/p"),
        ]))
        .unwrap();
        assert_eq!(path, PathBuf::from("/tmp/alt.toml"));
    }

    #[test]
    fn the_default_path_is_dot_config_postio() {
        let path = config_path_from(env_of(&[("HOME", "/home/p")])).unwrap();
        assert_eq!(path, PathBuf::from("/home/p/.config/postio/config.toml"));
    }

    #[test]
    fn without_any_environment_there_is_no_path() {
        assert!(matches!(
            config_dir_from(|_| None),
            Err(ConfigError::NoConfigDir)
        ));
    }
}
