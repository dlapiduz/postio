//! Settings, as a frontend reads and writes them.
//!
//! **Swift never parses or writes TOML** (ADR 0031). That is not a style
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
    /// stores to remember which pane was open. Empty for `Config file`, which
    /// is not one table but all of them.
    pub key: String,
    /// The nav label, and the pane's own title: "Sync & storage".
    pub label: String,
    /// Which heading this sits under.
    pub group: GroupFfi,
    /// The one line under the pane's title, saying what it is for.
    pub description: String,
    /// The bracketed table this pane writes — `[ui]` — for the footer that
    /// says where a change is going. `None` for the two panes that own no
    /// table of their own.
    pub table: Option<String>,
}

/// The two headings the nav groups its sections under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GroupFfi {
    /// Accounts, Filters, Composing.
    Mail,
    /// Appearance, Keyboard, Sync & storage, Privacy, Config file.
    Application,
}

impl From<postio_ui::settings::Group> for GroupFfi {
    fn from(group: postio_ui::settings::Group) -> Self {
        use postio_ui::settings::Group;
        match group {
            Group::Mail => GroupFfi::Mail,
            Group::Application => GroupFfi::Application,
        }
    }
}

impl GroupFfi {
    /// The sidebar heading, already upper-cased.
    pub fn label(self) -> &'static str {
        match self {
            GroupFfi::Mail => "MAIL",
            GroupFfi::Application => "APPLICATION",
        }
    }
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
            label: section.label().to_string(),
            group: section.group().into(),
            description: section.description().to_string(),
            table: section.table().map(str::to_string),
        })
        .collect()
}

/// The sidebar heading for a group, already upper-cased.
#[uniffi::export]
pub fn settings_group_label(group: GroupFfi) -> String {
    group.label().to_string()
}

/// `300` → `5 min`, `90` → `90s` — the sentence under Check for mail.
#[uniffi::export]
pub fn settings_humanize_interval(seconds: u64) -> String {
    postio_ui::settings::humanize_interval(seconds)
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

/// Where `config.toml` lives on this platform.
///
/// `postio_config::paths` already resolves this, per platform and per
/// `$XDG_CONFIG_HOME`, and a frontend that guessed would edit a file nothing
/// loads.
#[uniffi::export]
pub fn settings_path() -> Result<String, SettingsError> {
    postio_config::paths::config_path()
        .map(|path| path.display().to_string())
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })
}

/// The file at `path`, or an empty document when it is not there yet.
///
/// A first run has no `config.toml`, and that is not an error: the pane shows
/// defaults over an empty document and the file is created on the first save.
#[uniffi::export]
pub fn settings_load(path: String) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Save `text` to `path`, the way an editor saves.
///
/// Through `postio_config::save`, so the running application learns about the
/// change the same way it learns about an `$EDITOR` one — see that module for
/// why an in-place write would be invisible to the watcher.
#[uniffi::export]
pub fn settings_save(path: String, text: String) -> Result<(), SettingsError> {
    postio_config::save::write_atomically(std::path::Path::new(&path), &text).map_err(|err| {
        SettingsError::Invalid {
            message: err.to_string(),
        }
    })
}

/// Canvas 1b's row geometry for one density, in logical pixels.
///
/// The numbers are `postio_ui::row::Metrics`, which GTK draws by. Crossing
/// them rather than restating them in Swift is the difference between one
/// setting and two settings with one name in the file: a row that is 26px on
/// one platform and 34 on the other is not "compact" in both.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct RowMetricsFfi {
    /// Space above and below the row's content.
    pub pad_y: f32,
    /// How far in the content starts, accent edge included.
    pub inset: f32,
    /// The avatar chip, square.
    pub avatar: f32,
    /// Between the avatar and the text column.
    pub gap: f32,
    /// Between the sender line and the subject.
    pub subject_gap: f32,
    /// Between the snippet and the key hints the focused row reveals.
    pub hints_gap: f32,
    /// Whether the snippet line is drawn at all — `false` at the tightest
    /// density, which is the whole of what makes it the tightest.
    pub snippet: bool,
}

/// The geometry `density` asks for.
#[uniffi::export]
pub fn row_metrics(density: DensityFfi) -> RowMetricsFfi {
    let metrics = postio_ui::row::Metrics::for_density(density.into());
    RowMetricsFfi {
        pad_y: metrics.pad_y,
        inset: metrics.inset,
        avatar: metrics.avatar,
        gap: metrics.gap,
        subject_gap: metrics.subject_gap,
        hints_gap: metrics.hints_gap,
        snippet: metrics.snippet,
    }
}

/// The timestamp column for a row received at `received_at` (epoch seconds).
///
/// Takes the instant rather than answering once at row-build time, because
/// "today" moves: a list left open across midnight would otherwise keep
/// drawing `09:14` for a message that is now yesterday's. `postio_ui::row`
/// owns the rule — clock today, weekday this week, date beyond, year past it.
#[uniffi::export]
pub fn row_timestamp(received_at: i64) -> String {
    let received = chrono::DateTime::from_timestamp(received_at, 0).unwrap_or_default();
    postio_ui::row::timestamp(received, chrono::Local::now())
}

/// One key hint on the focused row: the key, and what it does.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowHintFfi {
    /// The key as the user would press it, from their own bindings.
    pub key: String,
    /// The verb, in the canvas' words — "reply", "archive".
    pub label: String,
}

/// One verb a row offers the mouse.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowActionFfi {
    /// The registry command it runs — `archive`, `flag`, `delete`.
    ///
    /// A command id, never a local implementation: a hover action that did
    /// its own thing would be a fourth way to archive that undo did not know
    /// about.
    pub command: String,
    /// What a screen reader calls it, and what the context menu says.
    pub title: String,
}

/// The three verbs triage is made of, left to right.
///
/// The same three the keyboard runs with `a`, `s` and `d`, and the same three
/// the bulk bar carries: one row or twenty, the mouse says the same thing.
#[uniffi::export]
pub fn row_actions() -> Vec<RowActionFfi> {
    postio_ui::row::RowAction::ALL
        .into_iter()
        .map(|action| RowActionFfi {
            command: action.command().as_str().to_string(),
            title: action.title().to_string(),
        })
        .collect()
}
