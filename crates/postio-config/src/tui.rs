use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Extras;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preview {
    #[default]
    Toggle,
    Split,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuiConfig {
    #[serde(default)]
    pub preview: Preview,
    #[serde(default = "yes")]
    pub mouse: bool,
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extras: Extras,
}

fn yes() -> bool {
    true
}

impl Default for TuiConfig {
    fn default() -> Self {
        TuiConfig {
            preview: Preview::default(),
            mouse: true,
            colors: BTreeMap::new(),
            extras: Extras::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn no_tui_section_means_the_defaults() {
        let config: Config = toml::from_str("").expect("parse");
        assert_eq!(config.tui, TuiConfig::default());
        assert_eq!(config.tui.preview, Preview::Toggle);
        assert!(config.tui.mouse);
    }

    #[test]
    fn the_section_and_its_colours_are_read() {
        let config: Config = toml::from_str(
            "[tui]\npreview = \"split\"\nmouse = false\n\n[tui.colors]\nflagged = \"#ff8800\"\n",
        )
        .expect("parse");
        assert_eq!(config.tui.preview, Preview::Split);
        assert!(!config.tui.mouse);
        assert_eq!(
            config.tui.colors.get("flagged").map(String::as_str),
            Some("#ff8800")
        );
    }
}
