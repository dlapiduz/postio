//! `[ui]` — appearance and interaction preferences.

use serde::{Deserialize, Serialize};

use crate::{Config, Extras};

/// Message-list row height. The PLATE design is airy (40px rows); the other two
/// tighten the same row anatomy rather than changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// 40px rows — the default, matching the chosen design direction.
    #[default]
    Airy,
    /// Middle setting.
    Comfortable,
    /// Tightest rows, most messages on screen.
    Compact,
}

/// Light/dark preference. `System` follows the desktop's color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the GNOME light/dark setting.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The `[ui]` section.
///
/// ```toml
/// [ui]
/// density = "airy"          # airy | comfortable | compact
/// theme = "system"          # system | light | dark
/// show_hover_actions = true # mouse parity: reveal row actions on hover
/// thread_drill = true       # `t` turns the list column into the thread
/// show_key_hints = true     # the focused row's own keyboard hints
/// sender_avatars = true     # initials chip per row, from canvas 1b
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    /// Message-list row height.
    #[serde(default)]
    pub density: Density,
    /// Light/dark preference.
    #[serde(default)]
    pub theme: Theme,
    /// Show per-row actions when the pointer is over a row.
    #[serde(default = "crate::yes")]
    pub show_hover_actions: bool,
    /// Let `t` drill the list column into the focused thread.
    #[serde(default = "crate::yes")]
    pub thread_drill: bool,
    /// Show the focused row's key hints (`e reply`, `a archive`, `t thread`).
    /// Off leaves every binding in force -- this only stops the row from
    /// naming them, for someone who already knows the keyboard (#422).
    #[serde(default = "crate::yes")]
    pub show_key_hints: bool,
    /// Show each row's sender-initials chip, per canvas 1b's row anatomy.
    #[serde(default = "crate::yes")]
    pub sender_avatars: bool,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: Density::default(),
            theme: Theme::default(),
            show_hover_actions: true,
            thread_drill: true,
            show_key_hints: true,
            sender_avatars: true,
            extra: Extras::new(),
        }
    }
}

#[cfg(test)]
mod tests {
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
        // Back-compat: a config.toml written before this field existed has
        // no `sender_avatars` key at all, and must still parse as "on" --
        // the same default a brand new file gets.
        let config = Config::from_toml_str("[ui]\ntheme = \"dark\"\n").expect("parses");
        assert!(config.ui.sender_avatars);
    }
}
