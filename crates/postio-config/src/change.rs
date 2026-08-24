//! What changed between two configurations.
//!
//! Reapplying everything on every keystroke would be visibly slow, so a
//! reload names the sections that actually moved: the keymap only rebuilds on
//! `keys`, the message list only re-measures its rows on `ui`, the sync
//! engine only re-plans on `sync`. A consumer that cares about one section
//! reads one field and ignores the rest.
//!
//! [`ConfigChanged::between`] is a pure comparison — it does not know about
//! the watcher or the event bus. The natural place to call it is right after
//! [`crate::live::LiveConfig::apply`] reports [`crate::live::Reload::Applied`],
//! with the configuration from just before the call still in hand:
//!
//! ```
//! use postio_config::change::ConfigChanged;
//! use postio_config::live::{LiveConfig, Reload};
//! use postio_config::validate;
//! use std::path::Path;
//!
//! let mut live = LiveConfig::new(Path::new("config.toml"));
//! let before = live.config().clone();
//! let reload = live.apply(validate::check_str("[ui]\ndensity = \"compact\"\n"));
//! let changed = match reload {
//!     Reload::Applied => ConfigChanged::between(&before, live.config()),
//!     _ => ConfigChanged::default(),
//! };
//! assert!(changed.ui);
//! assert!(!changed.keys);
//! ```

use serde::{Deserialize, Serialize};

use crate::Config;

/// Which sections of the configuration moved.
///
/// Consumers subscribe to what they care about: the keymap rebuilds on
/// `keys`, the list re-measures its rows on `ui`, the sync engine re-plans on
/// `sync`. Nothing else has to do anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChanged {
    /// `[ui]` — density, theme, hover actions, thread drill-in.
    pub ui: bool,
    /// `[keys]` — bindings, so the keymap must be rebuilt.
    pub keys: bool,
    /// `[accounts]` — a server, a login or a whole account changed.
    pub accounts: bool,
    /// `[sync]` — IDLE, polling, connection budget.
    pub sync: bool,
    /// `[filters]` — the saved queries in the sidebar.
    pub filters: bool,
    /// `[logging]` — the level, so a running app can be made louder without
    /// being restarted. This is the one section whose whole point is to be
    /// changed while something is going wrong.
    pub logging: bool,
}

impl ConfigChanged {
    /// Whether any subsystem needs to do anything at all.
    ///
    /// A save that only touches a top-level key this build does not
    /// recognize — preserved verbatim, never dropped — compares equal on
    /// every field above and must not repaint anything.
    pub fn any(&self) -> bool {
        self.ui || self.keys || self.accounts || self.sync || self.filters || self.logging
    }

    /// Compare two configurations section by section.
    ///
    /// Each field is a whole-section equality check rather than a per-key
    /// diff: a section is small and cheap to compare as a unit, and every
    /// consumer today reacts at section granularity anyway (rebuild the
    /// keymap, re-measure rows, re-plan sync). Changing one key inside a
    /// section — `[ui].density`, say — therefore reports only that section,
    /// which is the granularity consumers actually need.
    pub fn between(old: &Config, new: &Config) -> Self {
        ConfigChanged {
            ui: old.ui != new.ui,
            keys: old.keys != new.keys,
            accounts: old.accounts != new.accounts,
            sync: old.sync != new.sync,
            filters: old.filters != new.filters,
            logging: old.logging != new.logging,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> Config {
        Config::from_toml_str(toml).expect("valid")
    }

    #[test]
    fn changing_ui_density_touches_only_ui() {
        let old = config("[ui]\ndensity = \"compact\"\n");
        let new = config("[ui]\ndensity = \"comfortable\"\n");

        let changed = ConfigChanged::between(&old, &new);

        assert!(changed.ui);
        assert!(changed.any());
        assert!(!changed.keys);
        assert!(!changed.accounts);
        assert!(!changed.sync);
        assert!(!changed.filters);
    }

    #[test]
    fn rewriting_identical_content_reports_no_change() {
        let text = "[ui]\ndensity = \"compact\"\n\n[sync]\nidle = false\n";
        let old = config(text);
        let new = config(text);

        let changed = ConfigChanged::between(&old, &new);

        assert_eq!(changed, ConfigChanged::default());
        assert!(!changed.any());
    }

    #[test]
    fn each_section_is_reported_independently() {
        let old = Config::default();

        let new_keys = config("[keys]\narchive = \"y\"\n");
        assert_eq!(
            ConfigChanged::between(&old, &new_keys),
            ConfigChanged {
                keys: true,
                ..Default::default()
            }
        );

        let new_sync = config("[sync]\nidle = false\n");
        assert_eq!(
            ConfigChanged::between(&old, &new_sync),
            ConfigChanged {
                sync: true,
                ..Default::default()
            }
        );

        let new_filters = config("[filters.urgent]\nquery = \"is:flagged\"\n");
        assert_eq!(
            ConfigChanged::between(&old, &new_filters),
            ConfigChanged {
                filters: true,
                ..Default::default()
            }
        );

        let new_accounts = config(
            "[accounts.a]\nemail = \"ada@example.com\"\n\n[accounts.a.imap]\nhost = \"h\"\n",
        );
        assert_eq!(
            ConfigChanged::between(&old, &new_accounts),
            ConfigChanged {
                accounts: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn raising_the_log_level_is_a_change_a_subsystem_watches() {
        // The whole point of `[logging]` being a setting rather than only an
        // environment variable: turning it up has to reach a process that is
        // already running and already misbehaving.
        let old = Config::default();
        let new = config("[logging]\nlevel = \"debug\"\n");

        let changed = ConfigChanged::between(&old, &new);

        assert_eq!(
            changed,
            ConfigChanged {
                logging: true,
                ..Default::default()
            }
        );
        assert!(changed.any());
    }

    #[test]
    fn an_unknown_top_level_key_alone_changes_nothing_a_consumer_cares_about() {
        let old = config("[ui]\ndensity = \"compact\"\n");
        let new = config("[ui]\ndensity = \"compact\"\n\n[future_feature]\nenabled = true\n");

        let changed = ConfigChanged::between(&old, &new);

        assert!(
            !changed.any(),
            "an unrecognized section is not a section any subsystem watches"
        );
    }
}
