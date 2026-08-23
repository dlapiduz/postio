//! Postio's configuration: the schema for `~/.config/postio/config.toml` and
//! its typed deserialization.
//!
//! # The one source of truth
//!
//! There is no separate settings store. `config.toml` *is* the settings: the
//! settings panel edits this same file in place, `Ctrl+E` opens it in
//! `$EDITOR`, and a watcher re-parses it and applies the change live. That
//! shapes three rules the schema follows:
//!
//! 1. **A missing or empty file is not an error.** It yields working defaults,
//!    so first run needs nothing on disk.
//! 2. **Unknown keys survive a round trip.** People hand-edit this file, and a
//!    key written by a newer Postio (or a typo the user wants to see and fix)
//!    must not be silently deleted when the settings panel saves. Every section
//!    carries an `extra` passthrough for exactly this.
//! 3. **Secrets can never be stored here.** Passwords live in the Secret
//!    Service keyring; an account only references a keyring entry. Secret-
//!    looking keys are stripped on the way in *and* on the way out, and parser
//!    errors are redacted. See [`secrets`].
//!
//! Parsing is deliberately lenient: every field has a default, so a
//! half-written account still loads. Telling the user what is wrong in readable
//! prose is the job of [`validate`], which builds on this schema; [`live`]
//! keeps the last good configuration in force while the file is broken, and
//! [`watch`] re-runs both when the file changes underneath the app.
//!
//! ```
//! use postio_config::{Config, Density};
//!
//! let cfg = Config::from_toml_str(r#"
//!     [ui]
//!     density = "compact"
//!
//!     [accounts.icloud]
//!     email = "ada@example.com"
//!
//!     [accounts.icloud.imap]
//!     host = "imap.mail.me.com"
//! "#).unwrap();
//!
//! assert_eq!(cfg.ui.density, Density::Compact);
//! assert_eq!(cfg.ui.theme, postio_config::Theme::System); // untouched default
//! assert_eq!(cfg.account("icloud").unwrap().imap.port, 993);
//! assert_eq!(cfg.account("icloud").unwrap().imap_keyring_entry(), "postio:icloud:imap");
//! ```

#![warn(missing_docs)]

pub mod accounts;
pub mod error;
pub mod filters;
pub mod keys;
pub mod live;
pub mod paths;
pub mod secrets;
mod source;
pub mod sync;
pub mod ui;
pub mod validate;
pub mod watch;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

pub use accounts::{AccountConfig, AuthMethod, ImapConfig, MailSecurity, SmtpConfig};
pub use error::{ConfigError, Result};
pub use filters::FilterConfig;
pub use keys::KeyBindings;
pub use live::{LiveConfig, Reload};
pub use sync::{BodyFetch, SyncConfig};
pub use ui::{Density, Theme, UiConfig};
pub use validate::{Checked, ErrorKind, Validation, ValidationError};
pub use watch::{ConfigWatcher, WatchOptions};

/// Unknown keys preserved verbatim so a round trip never loses a hand-edit.
pub type Extras = Table;

/// `#[serde(default)]` helper for fields that default to `true`.
pub(crate) fn yes() -> bool {
    true
}

/// The whole of `config.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Config {
    /// `[ui]` — density, theme, hover actions, thread drill-in.
    #[serde(default)]
    pub ui: UiConfig,
    /// `[keys]` — command id to binding, overriding [`keys::DEFAULT_BINDINGS`].
    #[serde(default)]
    pub keys: KeyBindings,
    /// `[accounts]` — one table per account, keyed by a short id.
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
    /// `[sync]` — IDLE, polling, connection budget.
    #[serde(default)]
    pub sync: SyncConfig,
    /// `[filters]` — named saved queries.
    #[serde(default)]
    pub filters: BTreeMap<String, FilterConfig>,
    /// Top-level keys this version of Postio does not know.
    #[serde(flatten)]
    pub extra: Extras,

    /// Secret keys found in the file and dropped. Never serialized.
    #[serde(skip)]
    rejected_secrets: Vec<String>,
}

impl Config {
    /// Parse a TOML document.
    ///
    /// Secret-bearing keys are removed before deserialization, so no value here
    /// can ever hold a password; use [`Config::rejected_secrets`] to see what
    /// was dropped.
    pub fn from_toml_str(text: &str) -> Result<Self> {
        Self::parse(text, None)
    }

    /// Load `config.toml` from `path`.
    ///
    /// A missing file yields [`Config::default`] — first run needs nothing on
    /// disk. An unreadable or malformed file is an error.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        Self::parse(&text, Some(path))
    }

    /// Load `config.toml` from its standard location.
    ///
    /// See [`paths::config_path`]: `$POSTIO_CONFIG`, else
    /// `$XDG_CONFIG_HOME/postio/config.toml`, else `~/.config/postio/config.toml`.
    pub fn load() -> Result<Self> {
        Self::load_from_path(&paths::config_path()?)
    }

    fn parse(text: &str, path: Option<&Path>) -> Result<Self> {
        Self::parse_raw(text).map_err(|err| ConfigError::parse(path, &err))
    }

    /// The whole parse, keeping the `toml` error rather than flattening it.
    ///
    /// Validation needs the error's span and key path to point the validity
    /// line at the right character, which [`ConfigError::Parse`] deliberately
    /// does not carry.
    pub(crate) fn parse_raw(text: &str) -> std::result::Result<Self, toml::de::Error> {
        let mut table: Table = toml::from_str(text)?;
        let rejected_secrets = secrets::strip_secrets(&mut table);
        let mut config = Self::from_table(table)?;
        config.rejected_secrets = rejected_secrets;
        Ok(config)
    }

    /// Deserialize a table that has already had its secrets stripped.
    pub(crate) fn from_table(table: Table) -> std::result::Result<Self, toml::de::Error> {
        let mut config: Config = Value::Table(table).try_into()?;
        config.normalize();
        Ok(config)
    }

    /// Render back to TOML.
    ///
    /// Unknown keys are written back out; secret-bearing keys are stripped
    /// again on the way, so a secret cannot reach disk even if one was injected
    /// into an [`Extras`] map programmatically.
    pub fn to_toml_string(&self) -> Result<String> {
        let value = Value::try_from(self).map_err(|err| ConfigError::Serialize(err.to_string()))?;
        let Value::Table(mut table) = value else {
            return Err(ConfigError::Serialize(
                "the configuration did not serialize to a table".to_string(),
            ));
        };
        secrets::strip_secrets(&mut table);
        toml::to_string_pretty(&table).map_err(|err| ConfigError::Serialize(err.to_string()))
    }

    /// Write the configuration to `path`, creating parent directories.
    ///
    /// The file is written `0600` on Unix: it holds no secrets, but it does
    /// describe the user's accounts.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let text = self.to_toml_string()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(path, text).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |source| ConfigError::Write {
                    path: path.to_path_buf(),
                    source,
                },
            )?;
        }
        Ok(())
    }

    /// Secret keys that were present in the file and dropped, as dotted paths.
    ///
    /// The settings panel surfaces these so the user knows to move the value
    /// into the keyring instead of quietly losing it.
    pub fn rejected_secrets(&self) -> &[String] {
        &self.rejected_secrets
    }

    /// An account by its `[accounts.<id>]` table key.
    pub fn account(&self, id: &str) -> Option<&AccountConfig> {
        self.accounts.get(id)
    }

    /// The account flagged `default = true`, else the first one.
    pub fn default_account(&self) -> Option<&AccountConfig> {
        self.accounts
            .values()
            .find(|account| account.is_default)
            .or_else(|| self.accounts.values().next())
    }

    /// Fill in the derived fields that TOML does not carry.
    fn normalize(&mut self) {
        for (id, account) in self.accounts.iter_mut() {
            account.id = id.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_config_has_no_accounts_and_no_rejected_secrets() {
        let cfg = Config::default();
        assert!(cfg.accounts.is_empty());
        assert!(cfg.default_account().is_none());
        assert!(cfg.rejected_secrets().is_empty());
        assert!(cfg.keys.is_empty(), "the file holds overrides only");
    }

    #[test]
    fn saving_and_loading_a_file_is_lossless() {
        let dir = std::env::temp_dir().join(format!("postio-cfg-unit-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut cfg = Config::default();
        cfg.ui.density = Density::Compact;
        cfg.keys
            .overrides_mut()
            .insert("archive".into(), "x".into());
        cfg.save_to_path(&path).unwrap();

        assert_eq!(Config::load_from_path(&path).unwrap(), cfg);
        std::fs::remove_dir_all(&dir).ok();
    }
}
