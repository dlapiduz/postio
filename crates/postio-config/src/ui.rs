//! `[ui]` — appearance and interaction preferences.

use serde::{Deserialize, Serialize};

use crate::Extras;

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
            extra: Extras::new(),
        }
    }
}
