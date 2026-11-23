//! Settings > Contacts: the pane's words, the Suggestions switches and the sync section. Rust
//! decides what each switch stops; the Mac draws it and sends a click.

use postio_ui::contacts_settings::{
    Device, ModeChoice, SuggestionsView, SyncSection, SyncStatus, intro, sync_section,
};

use crate::Session;
use crate::focus_contacts_sync::{SyncAccountFfi, SyncPlanFfi, SyncScopeFfi, SyncScopeOptionFfi};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncModeFfi {
    ReadOnly,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStatusFfi {
    NotOn,
    Previewed {
        /// "Would add 2, update 1, ask about 0, delete 0".
        summary: String,
    },
    Declined,
    BroughtIn {
        /// "Brought 2 people into Postio. Nothing was written to Contacts."
        note: String,
    },
}

impl From<SyncStatusFfi> for SyncStatus {
    fn from(status: SyncStatusFfi) -> Self {
        match status {
            SyncStatusFfi::NotOn => Self::NotOn,
            SyncStatusFfi::Previewed { summary } => Self::Previewed(summary),
            SyncStatusFfi::Declined => Self::Declined,
            SyncStatusFfi::BroughtIn { note } => Self::BroughtIn(note),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncModeOptionFfi {
    /// "Two-way" or "Read only".
    pub label: String,
    pub mode: SyncModeFfi,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSectionFfi {
    pub heading: String,
    pub account: SyncAccountFfi,
    pub modes: Vec<SyncModeOptionFfi>,
    pub scopes_heading: String,
    pub scopes: Vec<SyncScopeOptionFfi>,
    pub scope_note: String,
    pub preview: String,
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
                bring_in: plan.bring_in.map(Into::into),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsPaneFfi {
    /// The first clause of the paragraph at the top, drawn bold.
    pub intro_lead: String,
    pub intro_rest: String,
    /// "Sync with" and "What syncs"; `None` where there is nothing to sync
    /// with.
    pub sync: Option<SyncSectionFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SuggestionRowFfi {
    pub title: String,
    pub detail: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SuggestionsViewFfi {
    pub heading: String,
    /// The four, in order; a click names one by its index here.
    pub rows: Vec<SuggestionRowFfi>,
    pub footer: String,
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
            bring_in: plan.bring_in.map(Into::into),
        });
        let words = intro(Device::Mac, true);
        ContactsPaneFfi {
            intro_lead: words.lead,
            intro_rest: words.rest,
            sync: Some(sync_section(scope.into(), mode.into(), &status.into(), plan).into()),
        }
    }

    /// The pane appeared: read which Suggestions switches are on. Answered by
    /// `FocusContactsSettings`.
    pub fn focus_contacts_settings_opened(&self) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsSettingsOpened);
    }

    /// A click on the Suggestions switch at `index`; the redrawn pane arrives as
    /// `FocusContactsSettings`.
    pub fn focus_contacts_suggestion(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ContactsSuggestion(index));
    }

    pub fn focus_contacts_settings(&self) -> Option<SuggestionsViewFfi> {
        self.focus_driver().contacts_settings().map(Into::into)
    }
}
