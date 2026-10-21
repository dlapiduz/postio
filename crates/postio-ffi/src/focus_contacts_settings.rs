//! Settings > Contacts across the boundary (specs/011-focus-contacts D7,
//! handoff screen 08): the pane's words, the Suggestions switches and the
//! sync section. Rust says everything and decides what each switch stops; the
//! Mac draws it and sends a click.

use postio_ui::contacts_settings::{
    Device, ModeChoice, SuggestionsView, SyncSection, SyncStatus, intro, sync_section,
};

use crate::Session;
use crate::focus_contacts_sync::{SyncAccountFfi, SyncPlanFfi, SyncScopeFfi, SyncScopeOptionFfi};

/// How a sync would treat the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncModeFfi {
    /// Pull into Postio only.
    ReadOnly,
    /// Both ways.
    TwoWay,
}

impl From<SyncModeFfi> for ModeChoice {
    fn from(mode: SyncModeFfi) -> Self {
        match mode {
            SyncModeFfi::ReadOnly => Self::ReadOnly,
            SyncModeFfi::TwoWay => Self::TwoWay,
        }
    }
}

impl From<SyncModeFfi> for postio_contact_sync::Mode {
    fn from(mode: SyncModeFfi) -> Self {
        match mode {
            SyncModeFfi::ReadOnly => Self::ReadOnly,
            SyncModeFfi::TwoWay => Self::TwoWay,
        }
    }
}

/// What the account row says about the last attempt.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStatusFfi {
    /// Nothing asked of the Mac yet.
    NotOn,
    /// A dry run was shown; its counts.
    Previewed {
        /// "Would add 2, update 1, ask about 0, delete 0".
        summary: String,
    },
    /// The Mac said no.
    Declined,
}

impl From<SyncStatusFfi> for SyncStatus {
    fn from(status: SyncStatusFfi) -> Self {
        match status {
            SyncStatusFfi::NotOn => Self::NotOn,
            SyncStatusFfi::Previewed { summary } => Self::Previewed(summary),
            SyncStatusFfi::Declined => Self::Declined,
        }
    }
}

/// One of the account row's two modes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncModeOptionFfi {
    /// "Two-way" or "Read only".
    pub label: String,
    /// Which.
    pub mode: SyncModeFfi,
    /// Whether it is chosen.
    pub on: bool,
}

/// "Sync with" and "What syncs".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSectionFfi {
    /// "Sync with".
    pub heading: String,
    /// The account row.
    pub account: SyncAccountFfi,
    /// Two-way and Read only.
    pub modes: Vec<SyncModeOptionFfi>,
    /// "What syncs".
    pub scopes_heading: String,
    /// The choices.
    pub scopes: Vec<SyncScopeOptionFfi>,
    /// What the chosen one means.
    pub scope_note: String,
    /// The preview button's words.
    pub preview: String,
    /// The dry run, once there is one.
    pub plan: Option<SyncPlanFfi>,
}

impl From<SyncSection> for SyncSectionFfi {
    fn from(section: SyncSection) -> Self {
        SyncSectionFfi {
            heading: section.heading,
            account: SyncAccountFfi {
                title: section.account.title,
                mode: section.account.mode,
                note: section.account.note,
            },
            modes: section
                .modes
                .into_iter()
                .map(|option| SyncModeOptionFfi {
                    label: option.label,
                    mode: match option.mode {
                        ModeChoice::ReadOnly => SyncModeFfi::ReadOnly,
                        ModeChoice::TwoWay => SyncModeFfi::TwoWay,
                    },
                    on: option.on,
                })
                .collect(),
            scopes_heading: section.scopes_heading,
            scopes: section
                .scopes
                .into_iter()
                .map(|option| SyncScopeOptionFfi {
                    label: option.label,
                    scope: option.scope.into(),
                    on: option.on,
                })
                .collect(),
            scope_note: section.scope_note,
            preview: section.preview,
            plan: section.plan.map(|plan| SyncPlanFfi {
                summary: plan.summary,
                lines: plan.lines,
                foot: plan.foot,
            }),
        }
    }
}

/// The pane's static words: its explainer and, where the app can sync, the
/// sync section.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsPaneFfi {
    /// The first clause of the paragraph at the top, drawn bold.
    pub intro_lead: String,
    /// The rest of that paragraph.
    pub intro_rest: String,
    /// "Sync with" and "What syncs"; `None` where there is nothing to sync
    /// with.
    pub sync: Option<SyncSectionFfi>,
}

/// One Suggestions switch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SuggestionRowFfi {
    /// Its title.
    pub title: String,
    /// What it does, in the state it is in.
    pub detail: String,
    /// Whether it is on.
    pub on: bool,
}

/// The Suggestions section.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SuggestionsViewFfi {
    /// "Suggestions".
    pub heading: String,
    /// The four, in order; a click names one by its index here.
    pub rows: Vec<SuggestionRowFfi>,
    /// Where the work runs and what leaves.
    pub footer: String,
    /// Why the last change did not happen.
    pub error: Option<String>,
}

impl From<SuggestionsView> for SuggestionsViewFfi {
    fn from(view: SuggestionsView) -> Self {
        SuggestionsViewFfi {
            heading: view.heading,
            rows: view
                .rows
                .into_iter()
                .map(|row| SuggestionRowFfi {
                    title: row.title,
                    detail: row.detail,
                    on: row.on,
                })
                .collect(),
            footer: view.footer,
            error: view.error,
        }
    }
}

#[uniffi::export]
impl Session {
    /// The pane's words for the state the person has left sync in.
    pub fn contacts_pane(
        &self,
        scope: SyncScopeFfi,
        mode: SyncModeFfi,
        status: SyncStatusFfi,
        plan: Option<SyncPlanFfi>,
    ) -> ContactsPaneFfi {
        let plan = plan.map(|plan| postio_ui::contacts_sync::PlanView {
            summary: plan.summary,
            lines: plan.lines,
            foot: plan.foot,
        });
        let words = intro(Device::Mac);
        ContactsPaneFfi {
            intro_lead: words.lead,
            intro_rest: words.rest,
            sync: Some(sync_section(scope.into(), mode.into(), &status.into(), plan).into()),
        }
    }

    /// The pane appeared: read which Suggestions switches are on. The answer
    /// arrives as `FocusContactsSettings`.
    pub fn focus_contacts_settings_opened(&self) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsSettingsOpened);
    }

    /// A click on the Suggestions switch at `index`. The redrawn pane arrives
    /// as `FocusContactsSettings` once the host has changed it.
    pub fn focus_contacts_suggestion(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsSuggestion(index));
    }

    /// The Suggestions section as it stands, once it has been read.
    pub fn focus_contacts_settings(&self) -> Option<SuggestionsViewFfi> {
        self.focus_driver().contacts_settings().map(Into::into)
    }
}
