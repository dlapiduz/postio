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
    /// `[ui]` — density, theme, hover actions, key hints.
    pub ui: bool,
    /// `[keys]` — bindings, so the keymap must be rebuilt.
    pub keys: bool,
    /// `[sync]` — IDLE, polling, connection budget.
    pub sync: bool,
    /// `[filters]` — the saved queries in the sidebar.
    pub filters: bool,
    /// `[[rules]]` — the filing rules, and the order they run in.
    ///
    /// Compared as a whole ordered array, so reordering two rules is a change
    /// even though the set is the same: the order *is* the meaning here (ADR
    /// 0008 Q4), unlike every other section, where it is a map or a scalar.
    pub rules: bool,
    /// `[logging]` — the level, so a running app can be made louder without
    /// being restarted. This is the one section whose whole point is to be
    /// changed while something is going wrong.
    pub logging: bool,
    /// `[compose]` — where a signature sits relative to a quote.
    pub compose: bool,
    /// `[storage]` — the disk ceiling, so lowering it can take effect without
    /// a restart. Raising it is what a user does when eviction has started
    /// costing them refetches, and they should not have to restart to stop it.
    pub storage: bool,
}

impl ConfigChanged {
    /// Whether any subsystem needs to do anything at all.
    ///
    /// A save that only touches a top-level key this build does not
    /// recognize — preserved verbatim, never dropped — compares equal on
    /// every field above and must not repaint anything.
    pub fn any(&self) -> bool {
        self.ui
            || self.keys
            || self.sync
            || self.filters
            || self.rules
            || self.logging
            || self.compose
            || self.storage
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
            sync: old.sync != new.sync,
            filters: old.filters != new.filters,
            rules: old.rules != new.rules,
            logging: old.logging != new.logging,
            compose: old.compose != new.compose,
            storage: old.storage != new.storage,
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
    fn editing_a_rule_is_a_change_the_engine_can_see() {
        // `[[rules]]` is live-reloadable like every other section here, and
        // a section missing from this struct reports "nothing changed" for
        // every edit to it -- so the rule the user just wrote never runs and
        // nothing says why.
        let old = config("[[rules]]\nname = \"r\"\nquery = \"from:ada\"\nactions = [\"flag\"]\n");
        let new = config("[[rules]]\nname = \"r\"\nquery = \"from:bob\"\nactions = [\"flag\"]\n");
        let changed = ConfigChanged::between(&old, &new);
        assert!(changed.rules);
        assert!(changed.any());
        assert!(!changed.filters, "and nothing else was disturbed");
        assert!(!changed.ui);
    }

    #[test]
    fn reordering_two_rules_is_a_change() {
        // The order *is* the meaning (ADR 0008 Q4), so two files with the
        // same rules in a different order are two different configurations.
        // A comparison that sorted, or that compared as a set, would miss it.
        let first = "[[rules]]\nname = \"a\"\nquery = \"from:ada\"\nactions = [\"flag\"]\n";
        let second = "[[rules]]\nname = \"b\"\nquery = \"from:bob\"\nactions = [\"trash\"]\n";
        let old = config(&format!("{first}\n{second}"));
        let new = config(&format!("{second}\n{first}"));
        assert!(ConfigChanged::between(&old, &new).rules);
    }

    #[test]
    fn changing_ui_density_touches_only_ui() {
        let old = config("[ui]\ndensity = \"compact\"\n");
        let new = config("[ui]\ndensity = \"comfortable\"\n");

        let changed = ConfigChanged::between(&old, &new);

        assert!(changed.ui);
        assert!(changed.any());
        assert!(!changed.keys);
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
            ConfigChanged::default()
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
