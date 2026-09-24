//! `[tui]` — how the terminal frontend looks and behaves.
//!
//! ```toml
//! [tui]
//! preview = "toggle"    # or "split": the composer's rendered preview
//! mouse = true          # false leaves the terminal's own text selection
//!
//! [tui.colors]
//! flagged = "#ff8800"   # any role, by name, palette number or #rrggbb
//! ```
//!
//! Keys stay under `[keys]`, keyed by command id, shared with the desktop app:
//! there is no terminal keymap to configure (Principle II). The colour roles
//! are the terminal frontend's own (`postio-tui`'s `theme`), which is where an
//! unknown role or an unreadable colour is reported -- this crate carries the
//! strings and does not guess at a list it does not own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Extras;

/// How the composer shows the message it would send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preview {
    /// One key swaps the editor for the preview and back.
    #[default]
    Toggle,
    /// The editor and the preview side by side.
    Split,
}

/// The `[tui]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuiConfig {
    /// How the composer's preview is shown.
    #[serde(default)]
    pub preview: Preview,
    /// Whether the terminal frontend takes the mouse.
    #[serde(default = "yes")]
    pub mouse: bool,
    /// Colour overrides by role name.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    /// Keys in `[tui]` this version of Postio does not know.
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
