//! Errors produced while locating, reading and parsing `config.toml`.
//!
//! Every message here is user-facing: it is what the settings panel's validity
//! line shows. Human-readable *validation* of a successfully parsed file (bad
//! ports, incomplete accounts, duplicate bindings) is a separate concern and
//! lives in [`crate::validate`] — this type stays limited to I/O and schema.

use std::path::{Path, PathBuf};

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ConfigError>;

/// Something went wrong loading or writing the configuration file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Neither `$XDG_CONFIG_HOME` nor `$HOME` is set, so there is no place to
    /// look for `config.toml`.
    #[error("cannot locate the config directory: neither $XDG_CONFIG_HOME nor $HOME is set")]
    NoConfigDir,

    /// The file exists but could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file we tried to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// The file we tried to write.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file is not valid TOML, or does not match the schema.
    ///
    /// The message is pre-redacted: any line that looks like it holds a secret
    /// is replaced before it can reach a log or the settings panel.
    #[error("{path} is not valid: {message}")]
    Parse {
        /// The file being parsed, or `<config>` when parsing a string.
        path: String,
        /// The redacted parser message.
        message: String,
    },

    /// The configuration file could not be watched for changes.
    ///
    /// The underlying watcher error is kept as text: `notify` is an
    /// implementation detail of [`crate::watch`] and does not belong in this
    /// crate's public API.
    #[error("cannot watch {path}: {message}")]
    Watch {
        /// The file or directory we tried to watch.
        path: PathBuf,
        /// What the file-watching backend said.
        message: String,
    },

    /// The in-memory configuration could not be turned back into TOML.
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
