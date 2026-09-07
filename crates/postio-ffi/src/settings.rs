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
use postio_config::compose::{ComposeConfig, SignaturePlacement, patch_compose};
use postio_config::filters::{FilterConfig, patch_filters};
use postio_config::sync::{AttachmentFetch, BodyFetch, CheckForMail, SyncConfig, patch_sync};
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
    /// Where this pane's settings actually live, in a phrase for the footer.
    ///
    /// Not derivable from `table`: three panes own no `config.toml` table,
    /// and a footer that then says nothing has stopped doing its job. The
    /// Accounts pane is the one that most needs telling — its settings are
    /// in the encrypted store, and a footer naming `config.toml` sends
    /// somebody to edit a file that does not describe their account.
    pub stored_in: String,
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

/// Where a signature sits relative to quoted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignaturePlacementFfi {
    /// Under what was written and above the quote.
    AboveQuote,
    /// Under everything, the quote included.
    BelowQuote,
}

/// The `[compose]` table's typed settings — the Composing pane's model.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ComposingFfi {
    /// Where the signature goes on a reply.
    pub signature_on_reply: SignaturePlacementFfi,
    /// Where the signature goes on a forward.
    pub signature_on_forward: SignaturePlacementFfi,
    /// Which editor `⌃⌘E` hands the draft to. Empty means the platform's own
    /// idea of what opens a text file (#1288).
    pub editor: String,
}

/// What the hand-off should do with the configured editor.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HandoffTargetFfi {
    /// Nothing is chosen: open it the way this platform opens a text file.
    PlatformDefault,
    /// Open it with this application.
    Application {
        /// The application's name, as the platform knows it.
        name: String,
    },
    /// A program that wants a terminal, which no frontend here can give it.
    NeedsTerminal {
        /// The program, and the `advice` that names it.
        name: String,
        /// What to tell the person who chose it.
        advice: String,
    },
}

/// What to do about the configured editor.
///
/// `is_application` is the frontend's answer to the one question only the
/// platform can settle — whether an application by that name exists here.
/// Everything that follows from it is decided in `postio_ui::handoff`, once,
/// so both frontends behave the same way about a name that is not one.
#[uniffi::export]
pub fn settings_handoff_target(configured: String, is_application: bool) -> HandoffTargetFfi {
    match postio_ui::handoff::target(&configured, is_application) {
        postio_ui::handoff::Target::PlatformDefault => HandoffTargetFfi::PlatformDefault,
        postio_ui::handoff::Target::Application(name) => HandoffTargetFfi::Application { name },
        postio_ui::handoff::Target::NeedsTerminal(name) => HandoffTargetFfi::NeedsTerminal {
            advice: postio_ui::handoff::terminal_advice(&name),
            name,
        },
    }
}

/// What the hand-off button should say: `Open in BBEdit`, or `Edit elsewhere`
/// when nothing is chosen.
#[uniffi::export]
pub fn settings_handoff_label(configured: String) -> String {
    postio_ui::handoff::button_label(&configured)
}

/// How Postio learns about new mail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CheckForMailFfi {
    /// The server pushes — `IDLE`, where it is offered.
    Idle,
    /// Ask on a timer, for folders and servers without it.
    Poll,
    /// Only when asked.
    Manual,
}

/// When message bodies are downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BodyFetchFfi {
    /// On opening the message.
    Lazy,
    /// With the headers, ahead of being asked.
    Eager,
}

/// When attachment payloads are downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AttachmentFetchFfi {
    /// When the message is opened.
    OnOpen,
    /// With the message.
    Eager,
    /// Never, until asked for one by name.
    Never,
}

/// The `[sync]` table's typed settings — the Sync & storage pane's model.
///
/// Deliberately not every key: `SyncConfig::extra` stays on the Rust side,
/// for the reason this module's own doc gives.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncingFfi {
    /// How Postio learns about new mail.
    pub check_for_mail: CheckForMailFfi,
    /// Polling interval for folders without `IDLE`, in seconds.
    pub poll_interval_secs: u64,
    /// Maximum simultaneous connections per account.
    pub max_connections: u8,
    /// Start a sync as soon as the application opens.
    pub sync_on_startup: bool,
    /// When bodies are downloaded.
    pub body_fetch: BodyFetchFfi,
    /// When attachment payloads are downloaded.
    pub attachment_fetch: AttachmentFetchFfi,
    /// How many messages the first sync reaches back for, newest first.
    pub initial_sync_messages: u32,
    /// Master switch for desktop notifications on new mail.
    pub notify: bool,
}

/// One saved search, as the Filters pane draws it.
///
/// The `key` is the `[filters.<key>]` identity and is **not** what the user
/// sees: #292 keeps the key stable and TOML-safe so a rename cannot orphan a
/// filter, and `name` is whatever they actually called it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilterFfi {
    /// The stable `[filters.<key>]` identity. Never shown as a label.
    pub key: String,
    /// What the user called it, or the key when nobody has renamed it.
    pub name: String,
    /// The search expression this filter runs.
    pub query: String,
    /// Whether it appears in the sidebar.
    pub pinned: bool,
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
            stored_in: section.stored_in().to_string(),
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

/// The Composing pane's values, or `None` when the file will not parse.
///
/// `None` rather than defaults, for the reason
/// [`settings_appearance`] gives: a form full of plausible settings that are
/// not the user's invites saving it over the file they were fixing.
#[uniffi::export]
pub fn settings_composing(text: String) -> Option<ComposingFfi> {
    let compose = Config::from_toml_str(&text).ok()?.compose;
    Some(ComposingFfi {
        signature_on_reply: compose.signature_on_reply.into(),
        signature_on_forward: compose.signature_on_forward.into(),
        editor: compose.editor,
    })
}

/// Write `composing` into `text`'s `[compose]` table, leaving the rest
/// verbatim.
#[uniffi::export]
pub fn settings_patch_composing(
    text: String,
    composing: ComposingFfi,
) -> Result<String, SettingsError> {
    let mut compose: ComposeConfig = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .compose;
    compose.signature_on_reply = composing.signature_on_reply.into();
    compose.signature_on_forward = composing.signature_on_forward.into();
    // Trimmed on the way in: a name with a space on the end is not one the
    // platform will find, and the difference is invisible in a text field.
    compose.editor = composing.editor.trim().to_owned();
    patch_compose(&text, &compose).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// The Sync & storage pane's values, or `None` when the file will not parse.
#[uniffi::export]
pub fn settings_syncing(text: String) -> Option<SyncingFfi> {
    let sync = Config::from_toml_str(&text).ok()?.sync;
    Some(SyncingFfi {
        check_for_mail: sync.check_for_mail.into(),
        poll_interval_secs: sync.poll_interval_secs,
        max_connections: sync.max_connections,
        sync_on_startup: sync.sync_on_startup,
        body_fetch: sync.body_fetch.into(),
        attachment_fetch: sync.attachment_fetch.into(),
        initial_sync_messages: sync.initial_sync_messages,
        notify: sync.notify,
    })
}

/// Write `syncing` into `text`'s `[sync]` table, leaving the rest verbatim.
#[uniffi::export]
pub fn settings_patch_syncing(text: String, syncing: SyncingFfi) -> Result<String, SettingsError> {
    let mut sync: SyncConfig = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .sync;
    sync.check_for_mail = syncing.check_for_mail.into();
    sync.poll_interval_secs = syncing.poll_interval_secs;
    sync.max_connections = syncing.max_connections;
    sync.sync_on_startup = syncing.sync_on_startup;
    sync.body_fetch = syncing.body_fetch.into();
    sync.attachment_fetch = syncing.attachment_fetch.into();
    sync.initial_sync_messages = syncing.initial_sync_messages;
    sync.notify = syncing.notify;
    patch_sync(&text, &sync).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// Every filter in `text`, in the order the sidebar shows them.
///
/// Empty for a file with no `[filters]` in it — which is different from a
/// file that will not parse, and the pane says so differently.
#[uniffi::export]
pub fn settings_filters(text: String) -> Option<Vec<FilterFfi>> {
    let filters = Config::from_toml_str(&text).ok()?.filters;
    let mut rows: Vec<(Option<u32>, String, FilterFfi)> = filters
        .into_iter()
        .map(|(key, filter)| {
            let name = filter.name.clone().unwrap_or_else(|| key.clone());
            (
                filter.order,
                key.clone(),
                FilterFfi {
                    key,
                    name,
                    query: filter.query,
                    pinned: filter.pinned,
                },
            )
        })
        .collect();
    // `None` sorts after everything that has an order, then by key — the
    // alphabetical order every filter had before reordering existed, so a
    // file nobody has reordered reads exactly as it used to.
    rows.sort_by(|left, right| {
        (left.0.is_none(), left.0, &left.1).cmp(&(right.0.is_none(), right.0, &right.1))
    });
    Some(rows.into_iter().map(|(_, _, filter)| filter).collect())
}

/// Write one filter's editable fields back into `text`.
///
/// One filter rather than the whole set, because that is what a pane edits
/// and because rewriting all of them to change one is how the fields this
/// build does not know get lost. The key is the identity and is never
/// written from here: renaming is `name`, and moving a filter to a new key
/// would orphan whatever refers to it (#292).
#[uniffi::export]
pub fn settings_patch_filter(text: String, filter: FilterFfi) -> Result<String, SettingsError> {
    let mut filters = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .filters;
    let Some(existing) = filters.get_mut(&filter.key) else {
        return Err(SettingsError::Invalid {
            message: format!("there is no filter called {}", filter.key),
        });
    };
    existing.query = filter.query;
    existing.pinned = filter.pinned;
    // Blank means "not renamed", which is what `None` says in the file — so
    // clearing the field restores the key as the label rather than saving an
    // empty name that draws as nothing.
    let name = filter.name.trim();
    existing.name = (!name.is_empty() && name != filter.key).then(|| name.to_owned());
    patch_filters(&text, &filters).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// Remove a filter entirely.
#[uniffi::export]
pub fn settings_remove_filter(text: String, key: String) -> Result<String, SettingsError> {
    let mut filters = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .filters;
    filters.remove(&key);
    patch_filters(&text, &filters).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

/// Add a filter under `key`, running `query`.
#[uniffi::export]
pub fn settings_add_filter(
    text: String,
    key: String,
    query: String,
) -> Result<String, SettingsError> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err(SettingsError::Invalid {
            message: "a filter needs a name".to_owned(),
        });
    }
    let mut filters = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .filters;
    if filters.contains_key(&key) {
        return Err(SettingsError::Invalid {
            message: format!("there is already a filter called {key}"),
        });
    }
    filters.insert(
        key,
        FilterConfig {
            query,
            ..FilterConfig::default()
        },
    );
    patch_filters(&text, &filters).map_err(|err| SettingsError::Invalid {
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

impl From<SignaturePlacement> for SignaturePlacementFfi {
    fn from(placement: SignaturePlacement) -> Self {
        match placement {
            SignaturePlacement::AboveQuote => SignaturePlacementFfi::AboveQuote,
            SignaturePlacement::BelowQuote => SignaturePlacementFfi::BelowQuote,
        }
    }
}

impl From<SignaturePlacementFfi> for SignaturePlacement {
    fn from(placement: SignaturePlacementFfi) -> Self {
        match placement {
            SignaturePlacementFfi::AboveQuote => SignaturePlacement::AboveQuote,
            SignaturePlacementFfi::BelowQuote => SignaturePlacement::BelowQuote,
        }
    }
}

impl From<CheckForMail> for CheckForMailFfi {
    fn from(value: CheckForMail) -> Self {
        match value {
            CheckForMail::Idle => CheckForMailFfi::Idle,
            CheckForMail::Poll => CheckForMailFfi::Poll,
            CheckForMail::Manual => CheckForMailFfi::Manual,
        }
    }
}

impl From<CheckForMailFfi> for CheckForMail {
    fn from(value: CheckForMailFfi) -> Self {
        match value {
            CheckForMailFfi::Idle => CheckForMail::Idle,
            CheckForMailFfi::Poll => CheckForMail::Poll,
            CheckForMailFfi::Manual => CheckForMail::Manual,
        }
    }
}

impl From<BodyFetch> for BodyFetchFfi {
    fn from(value: BodyFetch) -> Self {
        match value {
            BodyFetch::Lazy => BodyFetchFfi::Lazy,
            BodyFetch::Eager => BodyFetchFfi::Eager,
        }
    }
}

impl From<BodyFetchFfi> for BodyFetch {
    fn from(value: BodyFetchFfi) -> Self {
        match value {
            BodyFetchFfi::Lazy => BodyFetch::Lazy,
            BodyFetchFfi::Eager => BodyFetch::Eager,
        }
    }
}

impl From<AttachmentFetch> for AttachmentFetchFfi {
    fn from(value: AttachmentFetch) -> Self {
        match value {
            AttachmentFetch::OnOpen => AttachmentFetchFfi::OnOpen,
            AttachmentFetch::Eager => AttachmentFetchFfi::Eager,
            AttachmentFetch::Never => AttachmentFetchFfi::Never,
        }
    }
}

impl From<AttachmentFetchFfi> for AttachmentFetch {
    fn from(value: AttachmentFetchFfi) -> Self {
        match value {
            AttachmentFetchFfi::OnOpen => AttachmentFetch::OnOpen,
            AttachmentFetchFfi::Eager => AttachmentFetch::Eager,
            AttachmentFetchFfi::Never => AttachmentFetch::Never,
        }
    }
}
