use std::path::PathBuf;

use crate::error::{ConfigError, Result};

/// A parameter rather than a `#[cfg]` so both layouts are asserted from either host; the macOS
/// answer is the one most sessions cannot check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Freedesktop,
    Apple,
}

impl Platform {
    pub const fn host() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Apple
        } else {
            Platform::Freedesktop
        }
    }

    /// `Postio`, capitalised: the platform convention, and Finder lists it.
    pub fn apple_support_dir(home: &str) -> PathBuf {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Postio")
    }
}

pub const CONFIG_FILE_NAME: &str = "config.toml";

pub const CONFIG_PATH_ENV: &str = "POSTIO_CONFIG";

/// Inside a Flatpak the sandbox points every XDG variable at the app's own directory, which would
/// give the desktop and terminal packages a store and a config each. They share both, so the host's
/// value is read from `HOST_<name>`, which Flatpak sets.
pub fn shared_xdg<F>(env: &F, name: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let sandboxed = env("FLATPAK_ID").is_some_and(|id| !id.is_empty());
    let key = if sandboxed {
        format!("HOST_{name}")
    } else {
        name.to_owned()
    };
    env(&key).filter(|value| !value.is_empty())
}

/// `$XDG_CONFIG_HOME` still wins on Apple: someone who set it meant it, and it lets one
/// configuration be shared with a Linux VM.
pub fn config_dir_from<F>(env: F, platform: Platform) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(xdg) = shared_xdg(&env, "XDG_CONFIG_HOME") {
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

pub fn config_path_from<F>(env: F, platform: Platform) -> Result<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(explicit) = env(CONFIG_PATH_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    Ok(config_dir_from(env, platform)?.join(CONFIG_FILE_NAME))
}

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
    fn inside_a_flatpak_the_config_is_the_hosts() {
        let sandboxed = [
            ("FLATPAK_ID", "dev.postio.PostioTui"),
            (
                "XDG_CONFIG_HOME",
                "/home/p/.var/app/dev.postio.PostioTui/config",
            ),
            ("HOME", "/home/p"),
        ];
        assert_eq!(
            config_dir_from(env_of(&sandboxed), Platform::Freedesktop).unwrap(),
            PathBuf::from("/home/p/.config/postio")
        );
        let mut with_host = sandboxed.to_vec();
        with_host.push(("HOST_XDG_CONFIG_HOME", "/x/conf"));
        assert_eq!(
            config_dir_from(env_of(&with_host), Platform::Freedesktop).unwrap(),
            PathBuf::from("/x/conf/postio")
        );
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
