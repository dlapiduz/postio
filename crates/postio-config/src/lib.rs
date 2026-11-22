//! The schema for `config.toml`, which is the settings: the panel edits the same file and a watcher
//! applies changes live. A missing or empty file yields defaults; unknown keys survive a round
//! trip;
//! secrets can never be stored here (stripped on the way in and out, parser errors redacted).
//! `[accounts]` is retired (#470): it round-trips as an unknown table and `validate` reports it.

pub mod change;
pub mod compose;
mod contact_rules;
pub mod error;
pub mod filters;
pub mod focus;
pub mod focus_edit;
pub mod keys;
pub mod live;
pub mod logging;
pub mod mcp;
pub mod model;
pub mod paths;
pub mod reader;
pub mod save;
pub mod secrets;
mod source;
mod splice;
pub mod storage;
pub mod sync;
pub mod tui;
pub mod ui;
pub mod validate;
pub mod watch;

use std::collections::BTreeMap;
use std::path::Path;

use postio_model::{MailboxRole, RoleOverrides};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

pub use change::ConfigChanged;
pub use compose::{ComposeConfig, SignaturePlacement, patch_compose};
pub use contact_rules::{ContactRule, Movable, digest_address, movable, rule_of, setting_of};
pub use error::{ConfigError, Result};
pub use filters::{FilterConfig, patch_filters};
pub use focus::{
    AfterUnsubscribe, DigestRule, Due, DueError, FocusConfig, FocusFilter, FocusVault, NeverEntry,
    Reading, StopMarker, never_entry,
};
pub use keys::{KeyBindings, patch_keys};
pub use live::{LiveConfig, Reload};
pub use logging::{LogLevel, LoggingConfig};
pub use mcp::{McpAccount, McpConfig, revoke_all_mcp, revoke_mcp_account};
pub use model::{EndpointRefused, FocusModel, ModelEndpoint, ModelFeature, ModelProblem, Reach};
pub use reader::{ReaderConfig, ZOOM_STEPS, nearest_zoom, patch_reader, save_zoom};
pub use storage::StorageConfig;
pub use sync::{ALL_MESSAGES, AttachmentFetch, BodyFetch, CheckForMail, SyncConfig, patch_sync};
pub use tui::{Preview, TuiConfig};
pub use ui::{Density, Theme, UiConfig, patch_ui};
pub use validate::{Checked, ErrorKind, Validation, ValidationError};
pub use watch::{ConfigWatcher, WatchOptions};

/// `Inbox` is not overridable: IMAP names that folder itself (RFC 3501), so remapping it would
/// disagree
/// with every other client. `Regular` is the absence of a role.
pub(crate) fn overridable(role: MailboxRole) -> bool {
    !matches!(role, MailboxRole::Inbox | MailboxRole::Regular)
}

pub type Extras = Table;

pub(crate) fn yes() -> bool {
    true
}

const STARTER_HEADER: &str = "\
# Postio didn't find a config.toml here, so it wrote this one with its
# current defaults. Every key is optional -- delete a key, a section, or
# the whole file to fall back to Postio's default for it. Changes here
# apply live; no restart needed.
";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub keys: KeyBindings,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default, rename = "saved_searches")]
    pub filters: BTreeMap<String, FilterConfig>,
    /// A `String` rather than a parsed role: an unknown key has to survive to validation, which
    /// reports it
    /// with a line number; parsing here would drop a typo silently (see `Config::role_overrides`).
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub compose: ComposeConfig,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub reader: ReaderConfig,
    #[serde(default)]
    pub focus: FocusConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(flatten)]
    pub extra: Extras,

    /// Secret keys found in the file and dropped. Never serialized.
    #[serde(skip)]
    rejected_secrets: Vec<String>,
}

impl Config {
    /// Keys that are not roles are dropped, never guessed: a typo resolved to the nearest role
    /// would file
    /// mail somewhere nobody chose. `inbox` is dropped because IMAP names that folder itself.
    /// Validation reports both.
    pub fn role_overrides(&self) -> RoleOverrides {
        RoleOverrides::from_pairs(self.mailboxes.iter().filter_map(|(role, path)| {
            let role = MailboxRole::from_name(role)?;
            (overridable(role) && !path.trim().is_empty()).then_some((role, path.clone()))
        }))
    }

    /// Secret-bearing keys are removed before deserialization, so no value here can hold a
    /// password.
    pub fn from_toml_str(text: &str) -> Result<Self> {
        Self::parse(text, None)
    }

    /// A missing file yields the defaults; an unreadable or malformed file is an error.
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

    pub fn load() -> Result<Self> {
        Self::load_from_path(&paths::config_path()?)
    }

    fn parse(text: &str, path: Option<&Path>) -> Result<Self> {
        Self::parse_raw(text).map_err(|err| ConfigError::parse(path, &err))
    }

    /// Keeps the `toml` error: validation needs its span and key path to point the validity line at
    /// the
    /// right character.
    pub(crate) fn parse_raw(text: &str) -> std::result::Result<Self, toml::de::Error> {
        let mut table: Table = toml::from_str(text)?;
        let rejected_secrets = secrets::strip_secrets(&mut table);
        let mut config = Self::from_table(table)?;
        config.rejected_secrets = rejected_secrets;
        Ok(config)
    }

    pub(crate) fn from_table(table: Table) -> std::result::Result<Self, toml::de::Error> {
        let mut config: Config = Value::Table(table).try_into()?;
        config.normalize();
        Ok(config)
    }

    /// Secret-bearing keys are stripped again on the way out, so a secret cannot reach disk even if
    /// one
    /// was injected into an `Extras` map.
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

    /// Returns `Ok(false)` if a file already existed, untouched however it parses. Seeding changes
    /// discoverability, not behaviour: it gives `Ctrl+E` something to open.
    pub fn seed_if_missing(path: &Path) -> Result<bool> {
        match std::fs::metadata(path) {
            Ok(_) => Ok(false),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let text = format!("{STARTER_HEADER}\n{}", Self::default().to_toml_string()?);
                Self::write_text_to_path(&text, path)?;
                Ok(true)
            }
            Err(source) => Err(ConfigError::Write {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Creates parent directories and sets `0600` on Unix.
    pub fn write_text_to_path(text: &str, path: &Path) -> Result<()> {
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

    /// The settings panel surfaces these so the user moves the value into the keyring instead of
    /// losing it.
    pub fn rejected_secrets(&self) -> &[String] {
        &self.rejected_secrets
    }

    fn normalize(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_config_has_nothing_in_it_and_no_rejected_secrets() {
        let cfg = Config::default();
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
        Config::write_text_to_path(&cfg.to_toml_string().unwrap(), &path).unwrap();

        assert_eq!(Config::load_from_path(&path).unwrap(), cfg);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seeding_a_missing_path_writes_the_defaults_in_effect() {
        let dir =
            std::env::temp_dir().join(format!("postio-cfg-seed-missing-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::remove_dir_all(&dir).ok();

        let wrote = Config::seed_if_missing(&path).unwrap();

        assert!(wrote, "nothing was there, so this should have written one");
        assert_eq!(Config::load_from_path(&path).unwrap(), Config::default());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# Postio didn't find a config.toml"),
            "the seeded file should explain itself before the TOML starts"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seeding_an_existing_file_never_touches_it() {
        let dir =
            std::env::temp_dir().join(format!("postio-cfg-seed-existing-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        // Deliberately not valid Config TOML -- proves seed_if_missing
        // never parses or validates, only checks whether the path exists.
        std::fs::write(&path, "not valid toml at all {{{").unwrap();

        let wrote = Config::seed_if_missing(&path).unwrap();

        assert!(!wrote, "a file was already there");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not valid toml at all {{{",
            "the existing file must be untouched, even though it does not parse"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seeding_an_existing_empty_file_never_touches_it() {
        let dir =
            std::env::temp_dir().join(format!("postio-cfg-seed-empty-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "").unwrap();

        let wrote = Config::seed_if_missing(&path).unwrap();

        assert!(
            !wrote,
            "an empty file still counts as \"something is there\""
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        std::fs::remove_dir_all(&dir).ok();
    }
}
