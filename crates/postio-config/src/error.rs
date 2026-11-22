//! Every message is user-facing: the settings panel's validity line shows it.

use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot locate the config directory: neither $XDG_CONFIG_HOME nor $HOME is set")]
    NoConfigDir,

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The message is pre-redacted: a line that looks like it holds a secret is replaced before it
    /// reaches a log or the settings panel.
    #[error("{path} is not valid: {message}")]
    Parse { path: String, message: String },

    /// The watcher error is kept as text: `notify` is an implementation detail of `watch` and does
    /// not
    /// belong in the public API.
    #[error("cannot watch {path}: {message}")]
    Watch { path: PathBuf, message: String },

    #[error("cannot serialize the configuration: {0}")]
    Serialize(String),
}

impl ConfigError {
    pub(crate) fn parse(path: Option<&Path>, err: &dyn std::fmt::Display) -> Self {
        ConfigError::Parse {
            path: path.map_or_else(|| "<config>".to_string(), |p| p.display().to_string()),
            message: crate::secrets::redact_secret_lines(&err.to_string()),
        }
    }
}
