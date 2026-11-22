//! Swift never parses or writes TOML (ADR 0031): values cross the boundary and every write goes
//! back through `postio_config`'s `patch_*`.
//! A patch takes the file text back so it can re-read `[ui]`; `UiConfig::extra` (unknown keys)
//! cannot cross as typed fields and must not be dropped.

use postio_config::Config;
use postio_config::compose::{ComposeConfig, SignaturePlacement, patch_compose};
use postio_config::filters::{FilterConfig, patch_filters};
use postio_config::sync::{AttachmentFetch, BodyFetch, CheckForMail, SyncConfig, patch_sync};
use postio_config::ui::{Density, Theme, UiConfig, patch_ui};
use postio_config::validate;
use postio_ui::settings::Section;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsSectionFfi {
    pub key: String,
    pub label: String,
    pub group: GroupFfi,
    pub description: String,
    pub table: Option<String>,
    /// Not derivable from `table`: three panes own no table, and Accounts lives in the
    /// encrypted store, so a footer naming `config.toml` would send people to the wrong file.
    pub stored_in: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GroupFfi {
    Mail,
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
    pub fn label(self) -> &'static str {
        match self {
            GroupFfi::Mail => "MAIL",
            GroupFfi::Application => "APPLICATION",
        }
    }
}

/// The only feedback on whether typed text took effect, so "invalid" alone is not enough.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsStatusFfi {
    pub valid: bool,
    pub line: Option<u32>,
    pub message: String,
    pub status_line: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DensityFfi {
    Airy,
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ThemeFfi {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppearanceFfi {
    pub density: DensityFfi,
    pub theme: ThemeFfi,
    pub show_hover_actions: bool,
    pub sender_avatars: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignaturePlacementFfi {
    AboveQuote,
    BelowQuote,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ComposingFfi {
    pub signature_on_reply: SignaturePlacementFfi,
    pub signature_on_forward: SignaturePlacementFfi,
    pub editor: String,
}

/// Three answers, not two: an unknown name is a terminal program on macOS but a typo on
/// freedesktop, and calling a typo a terminal program is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FoundEditorFfi {
    Application,
    TerminalProgram,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HandoffTargetFfi {
    PlatformDefault,
    Application { name: String },
    NeedsTerminal { name: String, advice: String },
    Missing { name: String, advice: String },
}

#[uniffi::export]
pub fn settings_handoff_target(configured: String, found: FoundEditorFfi) -> HandoffTargetFfi {
    let found = match found {
        FoundEditorFfi::Application => postio_ui::handoff::Found::Application,
        FoundEditorFfi::TerminalProgram => postio_ui::handoff::Found::TerminalProgram,
        FoundEditorFfi::Nothing => postio_ui::handoff::Found::Nothing,
    };
    match postio_ui::handoff::target(&configured, found) {
        postio_ui::handoff::Target::PlatformDefault => HandoffTargetFfi::PlatformDefault,
        postio_ui::handoff::Target::Application(name) => HandoffTargetFfi::Application { name },
        postio_ui::handoff::Target::NeedsTerminal(name) => HandoffTargetFfi::NeedsTerminal {
            advice: postio_ui::handoff::terminal_advice(&name),
            name,
        },
        postio_ui::handoff::Target::Missing(name) => HandoffTargetFfi::Missing {
            advice: postio_ui::handoff::missing_advice(&name),
            name,
        },
    }
}

#[uniffi::export]
pub fn settings_handoff_label(configured: String) -> String {
    postio_ui::handoff::button_label(&configured)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CheckForMailFfi {
    Idle,
    Poll,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BodyFetchFfi {
    Lazy,
    Eager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AttachmentFetchFfi {
    OnOpen,
    Eager,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncingFfi {
    pub check_for_mail: CheckForMailFfi,
    pub poll_interval_secs: u64,
    pub max_connections: u8,
    pub sync_on_startup: bool,
    pub body_fetch: BodyFetchFfi,
    pub attachment_fetch: AttachmentFetchFfi,
    pub initial_sync_messages: u32,
    pub notify: bool,
    pub badge: bool,
    pub empty_junk_after_days: u32,
    pub empty_trash_after_days: u32,
}

/// `key` is the stable `[saved_searches.<key>]` identity, not what the user sees; `name` is (#292).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilterFfi {
    pub key: String,
    pub name: String,
    pub query: String,
    pub pinned: bool,
}

#[uniffi::export]
pub fn settings_sections() -> Vec<SettingsSectionFfi> {
    Section::ALL
        .into_iter()
        .filter(|section| section.shown_in(crate::FRONTEND))
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

#[uniffi::export]
pub fn settings_group_label(group: GroupFfi) -> String {
    group.label().to_string()
}

#[uniffi::export]
pub fn settings_humanize_interval(seconds: u64) -> String {
    postio_ui::settings::humanize_interval(seconds)
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AfterUnsubscribeFfi {
    pub heading: String,
    pub note: String,
    pub choices: Vec<String>,
    pub delete: bool,
}

#[uniffi::export]
pub fn settings_after_unsubscribe(text: String) -> Option<AfterUnsubscribeFfi> {
    use postio_ui::unsubscribe as words;
    let config = Config::from_toml_str(&text).ok()?;
    Some(AfterUnsubscribeFfi {
        heading: words::SETTING_HEADING.to_owned(),
        note: words::SETTING_NOTE.to_owned(),
        choices: words::SETTING_CHOICES
            .iter()
            .map(|action| words::sweep_verb(*action).to_owned())
            .collect(),
        delete: config.focus.after_unsubscribe == postio_config::AfterUnsubscribe::Delete,
    })
}

#[uniffi::export]
pub fn settings_patch_after_unsubscribe(
    text: String,
    delete: bool,
) -> Result<String, SettingsError> {
    let action = if delete {
        postio_config::AfterUnsubscribe::Delete
    } else {
        postio_config::AfterUnsubscribe::Archive
    };
    written(
        postio_config::focus_edit::set_after_unsubscribe(&text, action),
        text,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoEmptyChoiceFfi {
    pub days: u32,
    pub label: String,
}

#[uniffi::export]
pub fn settings_auto_empty_choices() -> Vec<AutoEmptyChoiceFfi> {
    postio_ui::auto_empty::CHOICES
        .iter()
        .map(|days| AutoEmptyChoiceFfi {
            days: *days,
            label: postio_ui::auto_empty::label(*days),
        })
        .collect()
}

#[uniffi::export]
pub fn settings_auto_empty_words() -> Vec<String> {
    vec![
        postio_ui::auto_empty::HEADING.to_owned(),
        postio_ui::auto_empty::NOTE.to_owned(),
    ]
}

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

/// `None` rather than defaults: a form of plausible values invites saving it over the file being
/// fixed.
#[uniffi::export]
pub fn settings_appearance(text: String) -> Option<AppearanceFfi> {
    let ui = Config::from_toml_str(&text).ok()?.ui;
    Some(AppearanceFfi {
        density: ui.density.into(),
        theme: ui.theme.into(),
        show_hover_actions: ui.show_hover_actions,
        sender_avatars: ui.sender_avatars,
    })
}

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
    ui.sender_avatars = appearance.sender_avatars;
    patch_ui(&text, &ui).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

#[uniffi::export]
pub fn settings_composing(text: String) -> Option<ComposingFfi> {
    let compose = Config::from_toml_str(&text).ok()?.compose;
    Some(ComposingFfi {
        signature_on_reply: compose.signature_on_reply.into(),
        signature_on_forward: compose.signature_on_forward.into(),
        editor: compose.editor,
    })
}

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
        badge: sync.badge,
        empty_junk_after_days: sync.empty_junk_after_days,
        empty_trash_after_days: sync.empty_trash_after_days,
    })
}

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
    sync.badge = syncing.badge;
    sync.empty_junk_after_days = syncing.empty_junk_after_days;
    sync.empty_trash_after_days = syncing.empty_trash_after_days;
    patch_sync(&text, &sync).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

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
    // `None` sorts last, then by key, so a file nobody has reordered keeps the old alphabetical
    // order.
    rows.sort_by(|left, right| {
        (left.0.is_none(), left.0, &left.1).cmp(&(right.0.is_none(), right.0, &right.1))
    });
    Some(rows.into_iter().map(|(_, _, filter)| filter).collect())
}

/// One filter, not the set: rewriting all of them loses fields this build does not know.
/// The key is never written from here; moving to a new key would orphan references (#292).
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
    // Blank means "not renamed" (`None` in the file), not an empty name that draws as nothing.
    let name = filter.name.trim();
    existing.name = (!name.is_empty() && name != filter.key).then(|| name.to_owned());
    patch_filters(&text, &filters).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

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

#[uniffi::export]
pub fn settings_reader_zoom(text: String) -> Option<u16> {
    Some(Config::from_toml_str(&text).ok()?.reader.zoom)
}

#[uniffi::export]
pub fn settings_zoom_steps() -> Vec<u16> {
    postio_config::ZOOM_STEPS.to_vec()
}

#[uniffi::export]
pub fn settings_patch_reader_zoom(text: String, zoom: u16) -> Result<String, SettingsError> {
    let mut reader = Config::from_toml_str(&text)
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })?
        .reader;
    reader.zoom = postio_config::nearest_zoom(zoom);
    postio_config::patch_reader(&text, &reader).map_err(|err| SettingsError::Invalid {
        message: err.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteringPageFfi {
    pub switch: String,
    pub on: bool,
    pub state: String,
    pub filtered: String,
    pub today: Option<String>,
    pub open: String,
    pub open_key: Option<String>,
    pub kept: String,
    pub keys: Vec<KeyHintFfi>,
    pub never_heading: String,
    pub guards: String,
    pub never: Vec<FilteringEntryFfi>,
    pub never_empty: String,
    pub stopped_heading: String,
    pub stopped: Vec<FilteringEntryFfi>,
    pub stopped_empty: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilteringEntryFfi {
    pub says: String,
    pub acts: bool,
    pub undo_label: String,
    pub undo: FilteringUndoFfi,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FilteringUndoFfi {
    Never { entry: String },
    Marker { sender: String, kind: String },
}

impl From<postio_ui::filtering::Listed> for FilteringEntryFfi {
    fn from(listed: postio_ui::filtering::Listed) -> Self {
        FilteringEntryFfi {
            says: listed.says,
            acts: listed.acts,
            undo_label: listed.undo.label().to_owned(),
            undo: match listed.undo {
                postio_ui::filtering::Undo::Never(entry) => FilteringUndoFfi::Never { entry },
                postio_ui::filtering::Undo::Marker { sender, kind } => {
                    FilteringUndoFfi::Marker { sender, kind }
                }
            },
        }
    }
}

/// Installed by a session: a person's pin is no longer in the file (ADR 0048).
pub(crate) struct ContactFilters {
    pub(crate) rules: Box<dyn Fn() -> Vec<postio_config::ContactRule> + Send + Sync>,
    pub(crate) take_back: Box<dyn Fn(&postio_ui::filtering::Undo) -> bool + Send + Sync>,
}

static CONTACT_FILTERS: std::sync::Mutex<Option<std::sync::Arc<ContactFilters>>> =
    std::sync::Mutex::new(None);

pub(crate) fn follow_contacts(filters: ContactFilters) {
    *CONTACT_FILTERS.lock().expect("never poisoned") = Some(std::sync::Arc::new(filters));
}

fn contact_filters() -> Option<std::sync::Arc<ContactFilters>> {
    CONTACT_FILTERS.lock().expect("never poisoned").clone()
}

#[uniffi::export]
pub fn settings_filtering(text: String, filtered_today: Option<u32>) -> Option<FilteringPageFfi> {
    use postio_ui::filtering as words;
    let config = Config::from_toml_str(&text).ok()?;
    let keymap = postio_core::Keymap::resolve_on(&config.keys, crate::PLATFORM);
    let focus = match contact_filters() {
        Some(filters) => config.focus.with_contacts(&(filters.rules)()),
        None => config.focus.clone(),
    };
    let page = words::page(&focus, filtered_today, &keymap);
    Some(FilteringPageFfi {
        switch: words::SWITCH.to_owned(),
        on: page.on,
        state: page.state,
        filtered: words::FILTERED.to_owned(),
        today: page.today,
        open: words::OPEN.to_owned(),
        open_key: keymap
            .binding(postio_core::CommandId::GoToFiltered)
            .map(str::to_owned),
        kept: words::KEPT.to_owned(),
        keys: page
            .keys
            .into_iter()
            .map(|hint| KeyHintFfi {
                key: hint.key,
                label: hint.label,
            })
            .collect(),
        never_heading: words::NEVER.to_owned(),
        guards: words::GUARDS.to_owned(),
        never: page.never.into_iter().map(Into::into).collect(),
        never_empty: page.never_empty,
        stopped_heading: words::STOPPED.to_owned(),
        stopped: page.stopped.into_iter().map(Into::into).collect(),
        stopped_empty: words::STOPPED_EMPTY.to_owned(),
    })
}

#[uniffi::export]
pub fn settings_patch_filtering(text: String, on: bool) -> Result<String, SettingsError> {
    written(postio_config::focus_edit::set_filtering(&text, on), text)
}

#[uniffi::export]
pub fn settings_take_back_filter(
    text: String,
    undo: FilteringUndoFfi,
) -> Result<String, SettingsError> {
    // A person's pin or marker is not in the file: it goes from the person.
    if let Some(filters) = contact_filters() {
        let focus = Config::from_toml_str(&text)
            .map(|config| config.focus.with_contacts(&(filters.rules)()))
            .ok();
        let theirs = match (&undo, &focus) {
            (FilteringUndoFfi::Never { entry }, Some(focus)) => {
                focus.contact_pinned(entry).is_some()
            }
            (FilteringUndoFfi::Marker { sender, kind }, Some(focus)) => {
                focus.contact_stopped(sender, kind).is_some()
            }
            _ => false,
        };
        if theirs {
            let undo = match undo {
                FilteringUndoFfi::Never { entry } => postio_ui::filtering::Undo::Never(entry),
                FilteringUndoFfi::Marker { sender, kind } => {
                    postio_ui::filtering::Undo::Marker { sender, kind }
                }
            };
            return if (filters.take_back)(&undo) {
                Ok(text)
            } else {
                Err(SettingsError::Invalid {
                    message: "Postio could not take that back".to_owned(),
                })
            };
        }
    }
    let edit = match &undo {
        FilteringUndoFfi::Never { entry } => {
            postio_config::focus_edit::set_never(&text, entry, false)
        }
        FilteringUndoFfi::Marker { sender, kind } => {
            postio_config::focus_edit::set_stop_marker(&text, sender, kind, false)
        }
    };
    written(edit, text)
}

fn written(
    edit: postio_config::Result<Option<String>>,
    text: String,
) -> Result<String, SettingsError> {
    match edit {
        Ok(Some(written)) => Ok(written),
        Ok(None) => Ok(text),
        Err(error) => Err(SettingsError::Invalid {
            message: error.to_string(),
        }),
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SettingsError {
    /// The write did not happen and the file is as it was. One variant: a frontend shows the
    /// message and leaves the screen alone.
    #[error("{message}")]
    Invalid { message: String },
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

/// Resolved by `postio_config::paths`; a frontend that guessed would edit a file nothing loads.
#[uniffi::export]
pub fn settings_path() -> Result<String, SettingsError> {
    config_path()
        .map(|path| path.display().to_string())
        .map_err(|err| SettingsError::Invalid {
            message: err.to_string(),
        })
}

/// Every path this crate hands out goes through here, so a demo cannot reach the installed file.
pub(crate) fn config_path() -> postio_config::Result<std::path::PathBuf> {
    match crate::demo::config_file() {
        Some(file) => Ok(file),
        None => postio_config::paths::config_path(),
    }
}

/// A missing file is an empty document, not an error: the first save creates it.
#[uniffi::export]
pub fn settings_load(path: String) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Through `postio_config::save`, so the running app sees the change as it sees an `$EDITOR` one.
#[uniffi::export]
pub fn settings_save(path: String, text: String) -> Result<(), SettingsError> {
    postio_config::save::write_atomically(std::path::Path::new(&path), &text).map_err(|err| {
        SettingsError::Invalid {
            message: err.to_string(),
        }
    })
}

/// From `postio_ui::row::Metrics`, in logical pixels, so both platforms share one geometry.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct RowMetricsFfi {
    pub pad_y: f32,
    pub inset: f32,
    pub avatar: f32,
    pub gap: f32,
    pub subject_gap: f32,
    pub snippet: bool,
}

#[uniffi::export]
pub fn row_metrics(density: DensityFfi) -> RowMetricsFfi {
    let metrics = postio_ui::row::Metrics::for_density(density.into());
    RowMetricsFfi {
        pad_y: metrics.pad_y,
        inset: metrics.inset,
        avatar: metrics.avatar,
        gap: metrics.gap,
        subject_gap: metrics.subject_gap,
        snippet: metrics.snippet,
    }
}

/// Takes the instant, not a string built at row-build time: "today" moves, so a list open across
/// midnight must not keep `09:14`.
#[uniffi::export]
pub fn row_timestamp(received_at: i64) -> String {
    let received = chrono::DateTime::from_timestamp(received_at, 0).unwrap_or_default();
    postio_ui::row::timestamp(received, chrono::Local::now())
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyHintFfi {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowActionFfi {
    /// A command id, never a local implementation, so undo knows about every action.
    pub command: String,
    pub title: String,
}

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
