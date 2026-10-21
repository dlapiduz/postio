//! Syncing with Contacts, in words (specs/011-focus-contacts S12, handoff
//! screen 08 and its sync section): the sheet that explains it, the dry run
//! that says what it would do, and the words other screens use for a contact
//! that syncs. A dry run only: every line says nothing was written.
//!
//! The plan has names in it; it is for the person to read and is never
//! logged. No toolkit here.

use postio_contact_sync::{Plan, Scope};
use postio_core::{CommandId, Keymap};

use crate::contact_edit::EditButton;

/// What the Mac's Contacts is called to the person.
pub const ACCOUNT_NAME: &str = "Contacts (iCloud)";

/// One choice of "What syncs".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeOption {
    /// Its words.
    pub label: String,
    /// Which it is.
    pub scope: ScopeChoice,
    /// Whether it is chosen.
    pub on: bool,
}

/// A scope, as a frontend names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeChoice {
    /// People.
    People,
    /// People and companies.
    PeopleAndCompanies,
    /// Everything saved.
    Everything,
}

impl From<ScopeChoice> for Scope {
    fn from(choice: ScopeChoice) -> Self {
        match choice {
            ScopeChoice::People => Self::People,
            ScopeChoice::PeopleAndCompanies => Self::PeopleAndCompanies,
            ScopeChoice::Everything => Self::Everything,
        }
    }
}

/// The sheet: before sync is on, the dry run it shows, or a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSheetView {
    /// "Sync with Contacts".
    pub title: String,
    /// The paragraphs under it.
    pub lines: Vec<String>,
    /// "Contacts on this Mac (iCloud)", while the choice is being made.
    pub account: Option<AccountRow>,
    /// "What syncs", with its choices; empty once there is a plan.
    pub scopes: Vec<ScopeOption>,
    /// What the chosen scope means.
    pub scope_note: Option<String>,
    /// The dry run, once there is one.
    pub plan: Option<PlanView>,
    /// The main button.
    pub primary: EditButton,
    /// The other.
    pub secondary: Option<EditButton>,
}

/// The one account row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    /// "Contacts on this Mac (iCloud)".
    pub title: String,
    /// "Read only", the mode.
    pub mode: String,
    /// "Read only until writes are reviewed".
    pub note: String,
}

/// What a dry run says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    /// "Would add 12, update 3, ask about 2, delete 0": counts only.
    pub summary: String,
    /// The plan, a line each, with names.
    pub lines: Vec<String>,
    /// "Read only until writes are reviewed. Postio changed nothing ..."
    pub foot: String,
}

/// The scope's words.
pub fn scope_options(chosen: ScopeChoice) -> Vec<ScopeOption> {
    [
        ("People", ScopeChoice::People),
        ("People and companies", ScopeChoice::PeopleAndCompanies),
        ("Everything saved", ScopeChoice::Everything),
    ]
    .into_iter()
    .map(|(label, scope)| ScopeOption {
        label: label.to_owned(),
        scope,
        on: scope == chosen,
    })
    .collect()
}

/// What `scope` keeps in Postio.
pub fn scope_note(scope: ScopeChoice) -> String {
    match scope {
        ScopeChoice::People => {
            "Saved people only. Companies, lists, newsletters and automated senders stay in Postio."
        }
        ScopeChoice::PeopleAndCompanies => {
            "Saved people and companies only. Lists, newsletters and automated senders stay in Postio so they don't fill your phone."
        }
        ScopeChoice::Everything => {
            "Everything you saved, lists and newsletters too. Unsaved contacts never sync."
        }
    }
    .to_owned()
}

fn done(keymap: &Keymap) -> EditButton {
    EditButton {
        label: "Done".to_owned(),
        key: keymap.binding(CommandId::Back).map(str::to_owned),
    }
}

/// The sheet before anything is asked of the Mac.
pub fn intro(chosen: ScopeChoice, keymap: &Keymap) -> SyncSheetView {
    SyncSheetView {
        title: "Sync with Contacts".to_owned(),
        lines: vec![
            "Your contacts live in Postio. Sync is optional: it keeps other apps and your phone up to date."
                .to_owned(),
            "Turning it on asks the Mac for access to Contacts, once. Then Postio says what a sync would do. Nothing is written until you have reviewed it."
                .to_owned(),
        ],
        account: Some(AccountRow {
            title: "Contacts on this Mac (iCloud)".to_owned(),
            mode: "Read only".to_owned(),
            note: "Read only until writes are reviewed".to_owned(),
        }),
        scopes: scope_options(chosen),
        scope_note: Some(scope_note(chosen)),
        plan: None,
        primary: EditButton {
            label: "Turn on and preview".to_owned(),
            key: Some(
                crate::hints::fixed(
                    "Return",
                    "Turn on and preview",
                    "Return is this sheet's one default button",
                )
                .key,
            ),
        },
        secondary: Some(EditButton {
            label: "Not now".to_owned(),
            key: keymap.binding(CommandId::Back).map(str::to_owned),
        }),
    }
}

/// How many plan lines the sheet shows before saying how many more.
const SHOWN: usize = 40;

/// What a dry run found.
pub fn plan_sheet(plan: &Plan, keymap: &Keymap) -> SyncSheetView {
    let mut lines = plan.report();
    if lines.len() > SHOWN {
        let more = lines.len() - SHOWN;
        lines.truncate(SHOWN);
        lines.push(format!("and {more} more"));
    }
    SyncSheetView {
        title: "Dry run: nothing was written".to_owned(),
        lines: Vec::new(),
        account: None,
        scopes: Vec::new(),
        scope_note: None,
        plan: Some(PlanView {
            summary: plan.summary(),
            lines,
            foot: "Read only until writes are reviewed. Postio changed nothing in Contacts and nothing in Postio."
                .to_owned(),
        }),
        primary: done(keymap),
        secondary: None,
    }
}

/// The sheet when the Mac said no.
pub fn denied(keymap: &Keymap) -> SyncSheetView {
    SyncSheetView {
        title: "Contacts access was declined".to_owned(),
        lines: vec![
            "Postio read nothing and changed nothing. To try again, allow Postio in System Settings, Privacy & Security, Contacts, then turn sync on again."
                .to_owned(),
        ],
        account: None,
        scopes: Vec::new(),
        scope_note: None,
        plan: None,
        primary: done(keymap),
        secondary: None,
    }
}

/// The words for a contact that syncs, once a sync has linked it: "saved in
/// Postio, then synced to Contacts (iCloud) and your other devices".
pub fn synced_bar_note(synced_with: Option<&str>) -> String {
    match synced_with {
        Some(account) => {
            format!("saved in Postio, then synced to {account} and your other devices")
        }
        None => "saved in Postio".to_owned(),
    }
}

/// "saved, synced with Contacts (iCloud)" for a card's role line and the merge
/// result.
pub fn synced_status(saved: bool, synced_with: Option<&str>) -> String {
    match (saved, synced_with) {
        (true, Some(account)) => format!("saved, synced with {account}"),
        (true, None) => "saved".to_owned(),
        (false, _) => "not saved".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use postio_contact_sync::{
        Authorization, ExternalRecord, FakeAddressBook, Fields, LocalRecord, Mode, dry_run,
    };
    use postio_model::ContactKind;

    use super::*;

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    fn fields(given: &str, address: &str) -> Fields {
        Fields {
            given: given.to_owned(),
            family: String::new(),
            emails: [address.to_owned()].into_iter().collect(),
            phones: Vec::new(),
            organization: String::new(),
            title: String::new(),
        }
    }

    #[test]
    fn the_intro_says_it_is_optional_read_only_and_asks_nothing_yet() {
        let sheet = intro(ScopeChoice::PeopleAndCompanies, &keymap());
        assert_eq!(sheet.title, "Sync with Contacts");
        assert!(sheet.lines[0].starts_with("Your contacts live in Postio. Sync is optional"));
        assert!(sheet.lines[1].contains("Nothing is written until you have reviewed it"));
        let account = sheet.account.expect("one account");
        assert_eq!(account.title, "Contacts on this Mac (iCloud)");
        assert_eq!(account.note, "Read only until writes are reviewed");
        let scopes: Vec<(&str, bool)> = sheet
            .scopes
            .iter()
            .map(|s| (s.label.as_str(), s.on))
            .collect();
        assert_eq!(
            scopes,
            [
                ("People", false),
                ("People and companies", true),
                ("Everything saved", false)
            ]
        );
        assert!(
            sheet
                .scope_note
                .unwrap()
                .contains("so they don't fill your phone")
        );
        assert_eq!(sheet.primary.label, "Turn on and preview");
        assert_eq!(sheet.primary.key.as_deref(), Some("Return"));
        assert_eq!(sheet.secondary.unwrap().label, "Not now");
        assert!(sheet.plan.is_none());
    }

    #[test]
    fn a_dry_run_says_nothing_was_written_in_counts_and_in_names() {
        let locals = vec![LocalRecord {
            id: 1,
            label: "Grace Oyelaran".into(),
            fields: fields("Grace", "grace@example.org"),
            saved: true,
            kind: ContactKind::Person,
        }];
        let mut book = FakeAddressBook::new(
            Authorization::Authorized,
            vec![ExternalRecord {
                id: "x1".into(),
                label: "Ada".into(),
                fields: fields("Ada", "ada@example.org"),
            }],
        );
        let plan = dry_run(
            &mut book,
            &locals,
            &[],
            Scope::PeopleAndCompanies,
            Mode::TwoWay,
        )
        .unwrap();
        let sheet = plan_sheet(&plan, &keymap());
        assert_eq!(sheet.title, "Dry run: nothing was written");
        let view = sheet.plan.expect("a plan");
        assert_eq!(view.summary, "Would add 1, update 0, ask about 0, delete 0");
        assert!(
            view.lines
                .iter()
                .any(|l| l == "Add Grace Oyelaran to Contacts"),
            "{:?}",
            view.lines
        );
        assert!(
            view.foot
                .contains("Postio changed nothing in Contacts and nothing in Postio")
        );
        assert_eq!(sheet.primary.label, "Done");
        assert_eq!(sheet.primary.key.as_deref(), Some("Escape"));
        assert_eq!(book.applied(), 0);
    }

    #[test]
    fn a_long_plan_says_how_many_more_instead_of_filling_the_sheet() {
        let locals: Vec<LocalRecord> = (0..60)
            .map(|n| LocalRecord {
                id: n,
                label: format!("Person {n}"),
                fields: fields(&format!("P{n}"), &format!("p{n}@example.org")),
                saved: true,
                kind: ContactKind::Person,
            })
            .collect();
        let plan = postio_contact_sync::plan(&locals, &[], &[], Scope::People, Mode::TwoWay);
        let view = plan_sheet(&plan, &keymap()).plan.unwrap();
        assert_eq!(view.lines.len(), 41);
        assert_eq!(view.lines.last().unwrap(), "and 20 more");
    }

    #[test]
    fn a_refusal_says_nothing_was_read_and_how_to_try_again() {
        let sheet = denied(&keymap());
        assert_eq!(sheet.title, "Contacts access was declined");
        assert!(sheet.lines[0].contains("read nothing and changed nothing"));
    }

    #[test]
    fn a_contact_that_syncs_says_so_and_one_that_does_not_stays_plain() {
        assert_eq!(synced_bar_note(None), "saved in Postio");
        assert_eq!(
            synced_bar_note(Some(ACCOUNT_NAME)),
            "saved in Postio, then synced to Contacts (iCloud) and your other devices"
        );
        assert_eq!(
            synced_status(true, Some(ACCOUNT_NAME)),
            "saved, synced with Contacts (iCloud)"
        );
        assert_eq!(synced_status(true, None), "saved");
        assert_eq!(synced_status(false, Some(ACCOUNT_NAME)), "not saved");
    }
}
