//! The configuration in force. A bad file never takes the running app down: the last configuration
//! that validated stays in force and the validity line explains what is wrong.

use std::path::{Path, PathBuf};

use crate::Config;
use crate::validate::{self, Checked, Validation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    Applied,
    Unchanged,
    Rejected,
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    path: PathBuf,
    config: Config,
    status: Validation,
    generation: u64,
}

impl LiveConfig {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            config: Config::default(),
            status: Validation::default(),
            generation: 0,
        }
    }

    /// Never fails: a first run has no file, and a broken file must still leave a working app to
    /// fix it in.
    pub fn load(path: &Path) -> Self {
        let mut live = Self::new(path);
        live.apply(validate::check_path(path));
        live
    }

    pub fn reload(&mut self) -> Reload {
        self.apply(validate::check_path(&self.path))
    }

    /// The watcher parses off the UI thread and hands the `Checked` over, so the UI thread only
    /// does this
    /// comparison.
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

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn status(&self) -> &Validation {
        &self.status
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bumped when a different configuration is applied, so a consumer can tell "touched" from
    /// "changed".
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
