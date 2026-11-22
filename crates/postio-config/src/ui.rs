use serde::{Deserialize, Serialize};

use crate::{Extras, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Airy,
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "crate::yes")]
    pub show_hover_actions: bool,
    #[serde(default = "crate::yes")]
    pub sender_avatars: bool,
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: Density::default(),
            theme: Theme::default(),
            show_hover_actions: true,
            sender_avatars: true,
            extra: Extras::new(),
        }
    }
}

/// The Appearance pane's write path (#873). `[ui]` is regenerated whole, so a comment on its header
/// or keys does not survive; `Config::to_toml_string` would drop comments anywhere.
pub fn patch_ui(text: &str, ui: &UiConfig) -> Result<String> {
    crate::splice::replace_section(text, "ui", &UiOnly { ui })
}

#[derive(Serialize)]
struct UiOnly<'a> {
    ui: &'a UiConfig,
}

#[cfg(test)]
mod tests {
    use crate::Config;

    use super::*;

    #[test]
    fn sender_avatars_round_trips_through_the_config_file() {
        let mut config = Config::default();
        config.ui.sender_avatars = false;

        let written = toml::to_string(&config).expect("serializes");
        let read = Config::from_toml_str(&written).expect("parses back");

        assert!(
            !read.ui.sender_avatars,
            "an explicit false must survive the round trip: {written}"
        );
    }

    #[test]
    fn sender_avatars_defaults_to_true_when_absent_from_an_existing_file() {
        let config = Config::from_toml_str("[ui]\ntheme = \"dark\"\n").expect("parses");
        assert!(config.ui.sender_avatars);
    }

    #[test]
    fn patch_ui_rewrites_only_the_ui_table_leaving_everything_else_verbatim() {
        let original = "\
# a hand-written comment nobody wants to lose
[saved_searches.old]
query = \"is:unread\"
pinned = true

[ui]
theme = \"system\" # inline comment, also not to be lost
density = \"airy\"
";
        let mut config = Config::from_toml_str(original).expect("parses");
        config.ui.theme = super::Theme::Dark;
        config.ui.density = super::Density::Compact;

        let patched = patch_ui(original, &config.ui).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "a comment outside [ui] must survive verbatim: {patched}"
        );
        assert!(
            patched.contains("[saved_searches.old]") && patched.contains("query = \"is:unread\""),
            "an unrelated section must survive untouched: {patched}"
        );

        let reparsed = Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.ui.theme, super::Theme::Dark);
        assert_eq!(reparsed.ui.density, super::Density::Compact);
        assert_eq!(reparsed.filters["old"].query, "is:unread");
    }

    #[test]
    fn patch_ui_adds_the_table_when_it_did_not_exist_before() {
        let original = "[sync]\ncheck_for_mail = \"poll\"\n";
        let config = Config::from_toml_str(original).expect("parses");

        let patched = patch_ui(original, &config.ui).expect("patches");
        assert!(patched.contains("[ui]"));
        assert!(patched.contains("check_for_mail = \"poll\""));
    }
}
