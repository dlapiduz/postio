//! The configuration that is actually in force, and what happens when the file
//! on disk stops making sense.
//!
//! `config.toml` is edited live — by the settings panel, by `$EDITOR`, by
//! anything. A save can therefore be observed mid-edit, half-written, or simply
//! wrong. The rule is that **a bad file never takes the running app down**: the
//! last configuration that validated stays in force and the validity line
//! explains what is wrong, so the user can fix it in place.
//!
//! ```
//! use postio_config::live::{LiveConfig, Reload};
//! use postio_config::validate;
//!
//! let mut live = LiveConfig::new(std::path::Path::new("config.toml"));
//! assert_eq!(
//!     live.apply(validate::check_str("[ui]\ndensity = \"compact\"\n")),
//!     Reload::Applied
//! );
//! // A broken edit is rejected, and the good one is still in force.
//! assert_eq!(live.apply(validate::check_str("[ui\n")), Reload::Rejected);
//! assert_eq!(live.config().ui.density, postio_config::Density::Compact);
//! assert!(!live.status().is_valid());
//! ```

use std::path::{Path, PathBuf};

use crate::Config;
use crate::validate::{self, Checked, Validation};

/// What a reload did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// The new configuration validated and differs from the old one.
    Applied,
    /// The new configuration validated and is identical — nothing to repaint.
    Unchanged,
    /// The new configuration was not usable; the last good one stays in force.
    Rejected,
}

/// The configuration in force, plus the status of the file it came from.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    path: PathBuf,
    config: Config,
    status: Validation,
    generation: u64,
}

impl LiveConfig {
    /// Start from defaults, without touching the disk.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            config: Config::default(),
            status: Validation::default(),
            generation: 0,
        }
    }

    /// Load `path`, falling back to defaults when it is missing or unusable.
    ///
    /// This never fails: a first run has no file, and a broken file must still
    /// leave the user with a working app to fix it in.
    pub fn load(path: &Path) -> Self {
        let mut live = Self::new(path);
        live.apply(validate::check_path(path));
        live
    }

    /// Re-read and re-validate the file.
    pub fn reload(&mut self) -> Reload {
        self.apply(validate::check_path(&self.path))
    }

    /// Adopt the result of a check that happened elsewhere.
    ///
    /// The watcher parses off the UI thread and hands the [`Checked`] over, so
    /// the only work on the UI thread is this comparison. It is also the seam
    /// the `ConfigChanged` diffing hangs off: `Applied` is exactly the moment a
    /// diff of the old and new configuration is worth computing.
    pub fn apply(&mut self, checked: Checked) -> Reload {
        self.status = checked.validation;
        let Some(config) = checked.config.filter(|_| self.status.is_valid()) else {
            return Reload::Rejected;
        };
        if config == self.config {
            return Reload::Unchanged;
        }
        self.config = config;
        self.generation += 1;
        Reload::Applied
    }

    /// The configuration in force.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The status of the file as last read — what the validity line shows.
    pub fn status(&self) -> &Validation {
        &self.status
    }

    /// The file being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bumped every time a *different* configuration is applied, so a consumer
    /// can tell "the file was touched" from "something actually changed".
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Density;

    fn live() -> LiveConfig {
        LiveConfig::new(Path::new("config.toml"))
    }

    #[test]
    fn a_fresh_live_config_is_the_defaults_and_is_valid() {
        let live = live();
        assert_eq!(live.config(), &Config::default());
        assert!(live.status().is_valid());
        assert_eq!(live.generation(), 0);
    }

    #[test]
    fn applying_the_same_text_twice_changes_nothing() {
        let mut live = live();
        let text = "[ui]\ndensity = \"compact\"\n";
        assert_eq!(live.apply(validate::check_str(text)), Reload::Applied);
        assert_eq!(live.generation(), 1);
        assert_eq!(live.apply(validate::check_str(text)), Reload::Unchanged);
        assert_eq!(live.generation(), 1);
    }

    #[test]
    fn a_semantic_error_is_rejected_even_though_it_parsed() {
        let mut live = live();
        live.apply(validate::check_str("[ui]\ndensity = \"compact\"\n"));
        let outcome = live.apply(validate::check_str("[accounts.a.imap]\nhost = \"h\"\n"));
        assert_eq!(outcome, Reload::Rejected);
        assert_eq!(live.config().ui.density, Density::Compact);
        assert!(!live.status().is_valid());
    }
}
