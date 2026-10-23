//! Syncing with Contacts across the boundary, as a dry run only
//! (specs/011-focus-contacts S12). The Mac moves bytes: it hands over a
//! snapshot of its side, and Rust decides everything and says it in words.
//! Nothing here writes to an address book, and access is asked for by the
//! Mac only when the person turns sync on.

use postio_contact_sync::{ExternalRecord, Fields, Scope};
use postio_ui::contacts_sync::{ScopeChoice, SyncSheetView};

use crate::Session;
use crate::focus_contact_edit::EditButtonFfi;

/// What syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncScopeFfi {
    /// People.
    People,
    /// People and companies.
    PeopleAndCompanies,
    /// Everything saved.
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

/// One record of the other side, as the Mac read it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExternalRecordFfi {
    /// The other side's identifier.
    pub id: String,
    /// Given name.
    pub given: String,
    /// Family name.
    pub family: String,
    /// Every address.
    pub emails: Vec<String>,
    /// Every phone number.
    pub phones: Vec<String>,
    /// Organisation.
    pub organization: String,
    /// Job title.
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

/// A choice of "What syncs".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncScopeOptionFfi {
    /// Its words.
    pub label: String,
    /// Which.
    pub scope: SyncScopeFfi,
    /// Whether it is chosen.
    pub on: bool,
}

/// The one account row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncAccountFfi {
    /// "Contacts on this Mac (iCloud)".
    pub title: String,
    /// "Read only".
    pub mode: String,
    /// "Read only until writes are reviewed".
    pub note: String,
}

/// What a dry run says.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncPlanFfi {
    /// Counts only.
    pub summary: String,
    /// The plan, a line each, with names.
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
    /// The button's words.
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

/// What bringing people in did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BringInResultFfi {
    /// What was done, in words: counts only.
    pub note: Option<String>,
    /// Why nothing was.
    pub error: Option<String>,
}

/// The sync sheet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSheetFfi {
    /// The title.
    pub title: String,
    /// The paragraphs.
    pub lines: Vec<String>,
    /// The account row.
    pub account: Option<SyncAccountFfi>,
    /// What syncs.
    pub scopes: Vec<SyncScopeOptionFfi>,
    /// What the chosen scope means.
    pub scope_note: Option<String>,
    /// The dry run.
    pub plan: Option<SyncPlanFfi>,
    /// The main button.
    pub primary: EditButtonFfi,
    /// The other.
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

/// A dry run's answer: the sheet, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncResultFfi {
    /// The dry run.
    pub sheet: Option<SyncSheetFfi>,
    /// Why not.
    pub error: Option<String>,
}

#[uniffi::export]
impl Session {
    /// The sheet before anything is asked of the Mac.
    pub fn contacts_sync_intro(&self, scope: SyncScopeFfi) -> SyncSheetFfi {
        postio_ui::contacts_sync::intro(scope.into(), &self.keymap()).into()
    }

    /// The sheet when the Mac said no.
    pub fn contacts_sync_denied(&self) -> SyncSheetFfi {
        postio_ui::contacts_sync::denied(&self.keymap()).into()
    }

    /// What a sync would do, from a snapshot of the Mac's side the person
    /// allowed it to read. A dry run: nothing is written to either side, and
    /// nothing in the answer reaches a log.
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

    /// Applies the Postio side of the plan the person was shown, under its
    /// `fingerprint`: saved contacts for the records only Contacts has, and
    /// the fields Contacts changed. Writes Postio's database and nothing
    /// else; the address book is never handed anything. The Contacts list
    /// reads the people again.
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
                // The cursor goes to the first new person, and the review
                // looks at what came in before anything else.
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
