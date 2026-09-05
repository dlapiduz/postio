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
//!    so first run needs nothing on disk to *function* — [`Config::seed_if_missing`]
//!    is a separate, later concern: Postio writes a starter file anyway, so
//!    there is something to find and edit rather than a blank buffer.
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
//! "#).unwrap();
//!
//! assert_eq!(cfg.ui.density, Density::Compact);
//! assert_eq!(cfg.ui.theme, postio_config::Theme::System); // untouched default
//! ```
//!
//! # Accounts are not configured here
//!
//! `[accounts.<id>]` used to describe an account's host, port, security and
//! display name, and nothing ever read it: those come from the store, written
//! once by onboarding. Editing the section saved, re-parsed without
//! complaint, and changed nothing about the running account, so it was
//! retired (#470). The section still round-trips as an unknown table, and
//! [`validate`] reports it so an edit cannot look like it worked.

pub mod change;
pub mod compose;
pub mod error;
pub mod filters;
pub mod keys;
pub mod live;
pub mod logging;
pub mod paths;
pub mod secrets;
mod source;
pub mod storage;
pub mod sync;
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
pub use error::{ConfigError, Result};
pub use filters::{FilterConfig, patch_filters};
pub use keys::{KeyBindings, patch_keys};
pub use live::{LiveConfig, Reload};
pub use logging::{LogLevel, LoggingConfig};
pub use storage::StorageConfig;
pub use sync::{AttachmentFetch, BodyFetch, CheckForMail, SyncConfig, patch_sync};
pub use ui::{Density, Theme, UiConfig, patch_ui};
pub use validate::{Checked, ErrorKind, Validation, ValidationError};
pub use watch::{ConfigWatcher, WatchOptions};

/// Whether a role is one a user may point at a folder of their choosing.
///
/// `Inbox` is not: IMAP names that folder itself (RFC 3501), so remapping it
/// would put Postio at odds with every other client on the account about
/// where mail arrives. `Regular` is not either — it is the absence of a role,
/// and "this folder is an ordinary folder" is what happens anyway.
///
/// Shared with [`validate`] so the section and its error
/// messages cannot disagree about what is allowed.
pub(crate) fn overridable(role: MailboxRole) -> bool {
    !matches!(role, MailboxRole::Inbox | MailboxRole::Regular)
}

/// Unknown keys preserved verbatim so a round trip never loses a hand-edit.
pub type Extras = Table;

/// `#[serde(default)]` helper for fields that default to `true`.
pub(crate) fn yes() -> bool {
    true
}

/// Prefixed onto a file [`Config::seed_if_missing`] writes. Every key below
/// is a real, in-effect default, not a documented reference of every
/// option — see the module docs for why a fuller generated reference is a
/// separate, later effort.
const STARTER_HEADER: &str = "\
# Postio didn't find a config.toml here, so it wrote this one with its
# current defaults. Every key is optional -- delete a key, a section, or
# the whole file to fall back to Postio's default for it. Changes here
# apply live; no restart needed.
";

/// The whole of `config.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Config {
    /// `[ui]` — density, theme, hover actions, key hints.
    #[serde(default)]
    pub ui: UiConfig,
    /// `[keys]` — command id to binding, overriding [`keys::DEFAULT_BINDINGS`].
    #[serde(default)]
    pub keys: KeyBindings,
    /// `[sync]` — IDLE, polling, connection budget.
    #[serde(default)]
    pub sync: SyncConfig,
    /// `[filters]` — named saved queries.
    #[serde(default)]
    pub filters: BTreeMap<String, FilterConfig>,
    /// `[mailboxes]` — role to the server's own folder path.
    ///
    /// Keyed by role and valued by path, the way `[keys]` is keyed by the
    /// thing you mean and valued by its spelling: a person knows what they
    /// want archived, and can read the folder's name off their own server.
    ///
    /// A `String` rather than a parsed role because an unknown key has to
    /// survive to validation, where it can be reported with a line number.
    /// Parsing here would drop a typo silently, which is the one thing this
    /// section must not do — see [`Config::role_overrides`].
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
    /// `[logging]` — how much Postio says about what it is doing.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// `[storage]` — how much disk the local store may use.
    #[serde(default)]
    pub storage: StorageConfig,
    /// `[compose]` — where a signature goes when a quote sits under it.
    #[serde(default)]
    pub compose: ComposeConfig,
    /// Top-level keys this version of Postio does not know.
    #[serde(flatten)]
    pub extra: Extras,

    /// Secret keys found in the file and dropped. Never serialized.
    #[serde(skip)]
    rejected_secrets: Vec<String>,
}

impl Config {
    /// `[mailboxes]` as the model's own override type.
    ///
    /// Keys that are not roles are **dropped, never guessed**. This cannot
    /// report a problem — validation does that, with the line number — and a
    /// typo silently resolved to the nearest role would file mail somewhere
    /// nobody chose. `inbox` is dropped for a different reason: IMAP names
    /// that folder itself in RFC 3501, so pointing it elsewhere would make
    /// Postio disagree with every other client on the same account about
    /// where mail arrives. Both are reported by
    /// [`validate`].
    pub fn role_overrides(&self) -> RoleOverrides {
        RoleOverrides::from_pairs(self.mailboxes.iter().filter_map(|(role, path)| {
            let role = MailboxRole::from_name(role)?;
            (overridable(role) && !path.trim().is_empty()).then_some((role, path.clone()))
        }))
    }

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

    /// Write a starter `config.toml` at `path` if nothing is there yet.
    ///
    /// Returns `Ok(true)` if it wrote one, `Ok(false)` if a file already
    /// existed there — untouched either way, however it parses or fails to.
    ///
    /// The seeded file is [`Config::default`] in effect: [`load_from_path`]
    /// on a missing path already yields the same defaults with no error, so
    /// this changes discoverability, not behaviour. It exists so `Ctrl+E`
    /// (or a file manager, or `cat`) has something to read and edit on first
    /// run, rather than opening a blank buffer that documents nothing.
    ///
    /// [`load_from_path`]: Config::load_from_path
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

    /// Writes already-rendered TOML text to `path`, creating parent
    /// directories and setting `0600` on Unix.
    ///
    /// Shared by [`Config::seed_if_missing`], which writes a starter file
    /// with a header prepended, and by [`patch_filters`]'s callers, which
    /// write a *patched* text rather than a freshly serialized `Config` —
    /// see that function's own doc for why a whole-struct reserialize is
    /// exactly what a structured edit here must not do.
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

    /// Secret keys that were present in the file and dropped, as dotted paths.
    ///
    /// The settings panel surfaces these so the user knows to move the value
    /// into the keyring instead of quietly losing it.
    pub fn rejected_secrets(&self) -> &[String] {
        &self.rejected_secrets
    }

    /// Fill in the derived fields that TOML does not carry.
    ///
    /// Nothing needs it now that `[accounts]` is retired (#470) -- it existed
    /// to copy each table's key onto the account it described. Kept as the
    /// hook the next derived field will want, rather than deleted and
    /// rediscovered.
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
