//! Settings, as a frontend reads and writes them.
//!
//! **Swift never parses or writes TOML** (ADR 0029). That is not a style
//! preference: `config.toml` is the settings store, it is a file a person
//! edits by hand, and a second writer with its own idea of key order and
//! comment survival would rewrite work nobody asked it to touch. So the
//! boundary carries *values*, and every write goes back through
//! `postio_config`'s `patch_*` functions — the same ones `postio-gtk` uses,
//! with the same format-preserving tests behind them.
//!
//! # Why a patch takes the text back
//!
//! [`settings_patch_appearance`] is handed the file, not just five fields,
//! and re-reads `[ui]` from it before writing. `UiConfig::extra` is a
//! `toml::Table` of keys this version of Postio does not know — a key from a
//! newer build, or one somebody added by hand — and it cannot cross a
//! boundary as typed fields. Reading it here means Swift cannot drop it,
//! because Swift never holds it.

use postio_config::Config;
use postio_config::ui::{Density, Theme, UiConfig, patch_ui};
use postio_config::validate;
use postio_ui::settings::Section;

/// One row of the settings nav.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsSectionFfi {
    /// Stable identifier — the `config.toml` table name, and what a frontend
    /// stores to remember which pane was open.
    pub key: String,
    /// The name a structured pane shows: "Appearance".
    pub title: String,
    /// The bracketed table name a text view shows: "[ui]".
    pub label: String,
}

/// The validity line along the foot of the settings surface.
///
/// Canvas 3f puts this where a dialog would put OK and Cancel, so it is the
/// only thing telling the user whether what they typed took effect. "invalid"
/// on its own would be the dead end canvas 3d forbids — hence the line.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsStatusFfi {
    /// Whether the configuration is usable as written.
    pub valid: bool,
    /// One-based line of the first problem, when there is one.
    pub line: Option<u32>,
    /// The first problem in plain language, or "valid".
    pub message: String,
    /// The whole footer, timing included: `valid · parsed in <1 ms`.
    pub status_line: String,
}

/// Message-list row height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DensityFfi {
    /// 40px rows — the default.
    Airy,
    /// Middle setting.
    Comfortable,
    /// Tightest rows, most messages on screen.
    Compact,
}

/// Light/dark preference. `System` follows the platform's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ThemeFfi {
    /// Follow the desktop's light/dark setting.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The `[ui]` table's typed settings — the Appearance pane's whole model.
///
/// Deliberately *not* every key in the table: `UiConfig::extra` stays on the
/// Rust side. See this module's own doc for why that is the point rather than
/// an omission.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppearanceFfi {
    /// Message-list row height.
    pub density: DensityFfi,
    /// Light/dark preference.
    pub theme: ThemeFfi,
    /// Show per-row actions when the pointer is over a row.
    pub show_hover_actions: bool,
    /// Show the focused row's key hints (`e reply`, `a archive`). Off leaves
    /// every binding in force — it only stops the row from naming them, for
    /// someone who already knows the keyboard (#422).
    pub show_key_hints: bool,
    /// Show each row's sender-initials chip, per canvas 1b's row anatomy.
    pub sender_avatars: bool,
}

/// Every settings section, in canvas 3f's nav order.
#[uniffi::export]
pub fn settings_sections() -> Vec<SettingsSectionFfi> {
    Section::ALL
        .into_iter()
        .map(|section| SettingsSectionFfi {
            key: section.key().to_string(),
            title: section.title().to_string(),
            label: section.label().to_string(),
        })
        .collect()
}

/// Validate `text` and describe it the way the footer shows it.
#[uniffi::export]
pub fn settings_status(text: String) -> SettingsStatusFfi {
    let validation = validate::check_str(&text).validation;
    SettingsStatusFfi {
        valid: validation.is_valid(),
        line: validation.first_error().map(|err| err.line as u32),
        message: validation.status(),
        status_line: validation.status_line(),
    }
}

/// The Appearance pane's values, or `None` when the file will not parse.
///
/// `None` rather than defaults, deliberately: a form full of plausible
/// settings that are not the user's invites them to save it over the file
/// they were trying to fix. The pane disables its controls instead, and the
/// footer says what is wrong.
#[uniffi::export]
pub fn settings_appearance(text: String) -> Option<AppearanceFfi> {
    let ui = Config::from_toml_str(&text).ok()?.ui;
    Some(AppearanceFfi {
        density: ui.density.into(),
        theme: ui.theme.into(),
        show_hover_actions: ui.show_hover_actions,
        show_key_hints: ui.show_key_hints,
        sender_avatars: ui.sender_avatars,
    })
}

/// Write `appearance` into `text`'s `[ui]` table, leaving the rest verbatim.
///
/// Takes the whole file because it re-reads `[ui]` to recover the keys this
/// version does not know before writing the table back.
#[uniffi::export]
pub fn settings_patch_appearance(
    text: String,
    appearance: AppearanceFfi,
) -> Result<String, SettingsError> {
    let mut ui: UiConfig = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .ui;
    ui.density = appearance.density.into();
    ui.theme = appearance.theme.into();
    ui.show_hover_actions = appearance.show_hover_actions;
    ui.show_key_hints = appearance.show_key_hints;
    ui.sender_avatars = appearance.sender_avatars;
    patch_ui(&text, &ui).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// Why a settings write could not be made.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SettingsError {
    /// The file does not parse, so there is no table to patch.
    #[error("{message}")]
    Invalid {
        /// What the parser said, for the footer.
        message: String,
    },
}

impl From<Density> for DensityFfi {
    fn from(density: Density) -> Self {
        match density {
            Density::Airy => DensityFfi::Airy,
            Density::Comfortable => DensityFfi::Comfortable,
            Density::Compact => DensityFfi::Compact,
        }
    }
}

impl From<DensityFfi> for Density {
    fn from(density: DensityFfi) -> Self {
        match density {
            DensityFfi::Airy => Density::Airy,
            DensityFfi::Comfortable => Density::Comfortable,
            DensityFfi::Compact => Density::Compact,
        }
    }
}

impl From<Theme> for ThemeFfi {
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::System => ThemeFfi::System,
            Theme::Light => ThemeFfi::Light,
            Theme::Dark => ThemeFfi::Dark,
        }
    }
}

impl From<ThemeFfi> for Theme {
    fn from(theme: ThemeFfi) -> Self {
        match theme {
            ThemeFfi::System => Theme::System,
            ThemeFfi::Light => Theme::Light,
            ThemeFfi::Dark => Theme::Dark,
        }
    }
}
