use serde::{Deserialize, Serialize};

use crate::Config;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChanged {
    pub ui: bool,
    pub keys: bool,
    pub sync: bool,
    pub filters: bool,
    /// A running app can be made louder without a restart.
    pub logging: bool,
    pub compose: bool,
    /// Lowering the ceiling takes effect without a restart.
    pub storage: bool,
    pub reader: bool,
    pub focus: bool,
}

impl ConfigChanged {
    /// A save that only touches a top-level key this build does not recognise compares equal on
    /// every
    /// field and must not repaint anything.
    pub fn any(&self) -> bool {
        self.ui
            || self.keys
            || self.sync
            || self.filters
            || self.logging
            || self.compose
            || self.storage
            || self.reader
            || self.focus
    }

    /// Whole-section equality: every consumer reacts at section granularity.
    pub fn between(old: &Config, new: &Config) -> Self {
        ConfigChanged {
            ui: old.ui != new.ui,
            keys: old.keys != new.keys,
            sync: old.sync != new.sync,
            filters: old.filters != new.filters,
            logging: old.logging != new.logging,
            compose: old.compose != new.compose,
            storage: old.storage != new.storage,
            reader: old.reader != new.reader,
            focus: old.focus != new.focus,
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
        assert!(!changed.sync);
        assert!(!changed.filters);
    }

    #[test]
    fn changing_focus_touches_only_focus() {
        let old = config("[focus]\nfiltering = true\n");
        let new = config("[focus]\nfiltering = false\n");

        let changed = ConfigChanged::between(&old, &new);

        assert!(changed.focus);
        assert!(changed.any());
        assert!(!changed.keys);
        assert!(!changed.ui);
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

        let new_filters = config("[saved_searches.urgent]\nquery = \"is:flagged\"\n");
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
