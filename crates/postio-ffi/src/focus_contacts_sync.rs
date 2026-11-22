//! Syncing with Contacts, as a dry run only: the Mac hands over a snapshot of its side, Rust
//! decides and says it.
//! Nothing writes to an address book, and the Mac asks for access only when the person turns sync
//! on.

use postio_contact_sync::{ExternalRecord, Fields, Scope};
use postio_ui::contacts_sync::{ScopeChoice, SyncSheetView};

use crate::Session;
use crate::focus_contact_edit::EditButtonFfi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncScopeFfi {
    People,
    PeopleAndCompanies,
    Everything,
}

impl From<SyncScopeFfi> for ScopeChoice {
    fn from(scope: SyncScopeFfi) -> Self {
        match scope {
            SyncScopeFfi::People => Self::People,
            SyncScopeFfi::PeopleAndCompanies => Self::PeopleAndCompanies,
            SyncScopeFfi::Everything => Self::Everything,
        }
    }
}

impl From<ScopeChoice> for SyncScopeFfi {
    fn from(scope: ScopeChoice) -> Self {
        match scope {
            ScopeChoice::People => Self::People,
            ScopeChoice::PeopleAndCompanies => Self::PeopleAndCompanies,
            ScopeChoice::Everything => Self::Everything,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExternalRecordFfi {
    /// The other side's identifier.
    pub id: String,
    pub given: String,
    pub family: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub organization: String,
    pub title: String,
}

impl From<ExternalRecordFfi> for ExternalRecord {
    fn from(record: ExternalRecordFfi) -> Self {
        let label = format!("{} {}", record.given.trim(), record.family.trim())
            .trim()
            .to_owned();
        ExternalRecord {
            id: record.id,
            label: if !label.is_empty() {
                label
            } else if !record.organization.trim().is_empty() {
                record.organization.trim().to_owned()
            } else {
                record.emails.first().cloned().unwrap_or_default()
            },
            fields: Fields {
                given: record.given,
                family: record.family,
                emails: record
                    .emails
                    .into_iter()
                    .map(|e| e.to_lowercase())
                    .collect(),
                phones: record.phones,
                organization: record.organization,
                title: record.title,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncScopeOptionFfi {
    pub label: String,
    pub scope: SyncScopeFfi,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncAccountFfi {
    pub title: String,
    pub mode: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPlanFfi {
    pub summary: String,
    pub lines: Vec<String>,
    /// "Read only until writes are reviewed. ..."
    pub foot: String,
    /// The button that brings Contacts-only people into Postio, when the plan
    /// has any to bring.
    pub bring_in: Option<BringInFfi>,
}

/// "Bring 12 into Postio", bound to the plan it was drawn under.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BringInFfi {
    pub label: String,
    /// The plan's fingerprint, handed back with the click.
    pub fingerprint: u64,
}

impl From<postio_ui::contacts_sync::BringIn> for BringInFfi {
    fn from(bring: postio_ui::contacts_sync::BringIn) -> Self {
        Self {
            label: bring.label,
            fingerprint: bring.fingerprint,
        }
    }
}

impl From<BringInFfi> for postio_ui::contacts_sync::BringIn {
    fn from(bring: BringInFfi) -> Self {
        Self {
            label: bring.label,
            fingerprint: bring.fingerprint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BringInResultFfi {
    /// What was done, in words: counts only.
    pub note: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSheetFfi {
    pub title: String,
    pub lines: Vec<String>,
    pub account: Option<SyncAccountFfi>,
    pub scopes: Vec<SyncScopeOptionFfi>,
    pub scope_note: Option<String>,
    pub plan: Option<SyncPlanFfi>,
    pub primary: EditButtonFfi,
    pub secondary: Option<EditButtonFfi>,
}

impl From<SyncSheetView> for SyncSheetFfi {
    fn from(sheet: SyncSheetView) -> Self {
        let button = |button: postio_ui::contact_edit::EditButton| EditButtonFfi {
            label: button.label,
            key: button.key,
        };
        SyncSheetFfi {
            title: sheet.title,
            lines: sheet.lines,
            account: sheet.account.map(|account| SyncAccountFfi {
                title: account.title,
                mode: account.mode,
                note: account.note,
            }),
            scopes: sheet
                .scopes
                .into_iter()
                .map(|option| SyncScopeOptionFfi {
                    label: option.label,
                    scope: option.scope.into(),
                    on: option.on,
                })
                .collect(),
            scope_note: sheet.scope_note,
            plan: sheet.plan.map(|plan| SyncPlanFfi {
                summary: plan.summary,
                lines: plan.lines,
                foot: plan.foot,
                bring_in: plan.bring_in.map(Into::into),
            }),
            primary: button(sheet.primary),
            secondary: sheet.secondary.map(button),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncResultFfi {
    pub sheet: Option<SyncSheetFfi>,
    pub error: Option<String>,
}

#[uniffi::export]
impl Session {
    pub fn contacts_sync_intro(&self, scope: SyncScopeFfi) -> SyncSheetFfi {
        postio_ui::contacts_sync::intro(scope.into(), &self.keymap()).into()
    }

    pub fn contacts_sync_denied(&self) -> SyncSheetFfi {
        postio_ui::contacts_sync::denied(&self.keymap()).into()
    }

    /// What a sync would do, from a snapshot of the Mac's side the person allowed it to read. A dry
    /// run: nothing is
    /// written, and nothing reaches a log.
    pub fn contacts_sync_plan(
        &self,
        records: Vec<ExternalRecordFfi>,
        scope: SyncScopeFfi,
        mode: crate::focus_contacts_settings::SyncModeFfi,
    ) -> SyncResultFfi {
        let Some(client) = self.client() else {
            return SyncResultFfi {
                sheet: None,
                error: Some("Postio is still starting".to_owned()),
            };
        };
        let records: Vec<ExternalRecord> = records.into_iter().map(Into::into).collect();
        let scope: ScopeChoice = scope.into();
        match crate::session::blocking(client.contacts_sync_plan(
            records,
            Scope::from(scope),
            mode.into(),
        )) {
            Ok(plan) => SyncResultFfi {
                sheet: Some(postio_ui::contacts_sync::plan_sheet(&plan, &self.keymap()).into()),
                error: None,
            },
            Err(error) => SyncResultFfi {
                sheet: None,
                error: Some(error.to_string()),
            },
        }
    }

    /// Applies the Postio side of the plan shown, under its `fingerprint`: saved contacts for the
    /// records only Contacts
    /// has, and the fields Contacts changed. Writes Postio's database and nothing else.
    pub fn contacts_sync_bring_in(
        &self,
        records: Vec<ExternalRecordFfi>,
        scope: SyncScopeFfi,
        mode: crate::focus_contacts_settings::SyncModeFfi,
        fingerprint: u64,
    ) -> BringInResultFfi {
        let Some(client) = self.client() else {
            return BringInResultFfi {
                note: None,
                error: Some("Postio is still starting".to_owned()),
            };
        };
        let records: Vec<ExternalRecord> = records.into_iter().map(Into::into).collect();
        let scope: ScopeChoice = scope.into();
        match crate::session::blocking(client.contacts_sync_bring_in(
            records,
            Scope::from(scope),
            mode.into(),
            fingerprint,
        )) {
            Ok(done) => {
                let ids = |ids: &[i64]| -> Vec<postio_model::ContactId> {
                    ids.iter()
                        .copied()
                        .map(postio_model::ContactId::new)
                        .collect()
                };
                let made = ids(&done.made);
                let mut touched = made.clone();
                touched.extend(ids(&done.touched));
                // The cursor goes to the first new person.
                self.focus_driver()
                    .input(postio_focus::Input::ContactsImported { made, touched });
                BringInResultFfi {
                    note: Some(postio_ui::contacts_sync::brought_in_note(
                        done.people,
                        done.fields,
                        done.skipped,
                    )),
                    error: None,
                }
            }
            Err(error) => BringInResultFfi {
                note: None,
                error: Some(error.to_string()),
            },
        }
    }
}
