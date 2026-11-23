//! Settings > Contacts in words, with no toolkit. Sync is the Mac's until Linux has an
//! address book: a pane made without it omits "Sync with" and "What syncs".

use postio_model::{ContactSuggestions, Suggestion};

use crate::contacts_sync::PlanView;
use crate::contacts_sync::{AccountRow, ScopeChoice, ScopeOption, scope_note, scope_options};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Mac,
    Computer,
}

impl Device {
    fn phrase(self) -> &'static str {
        match self {
            Self::Mac => "this Mac",
            Self::Computer => "this computer",
        }
    }
}

pub const TITLE: &str = "Contacts";

pub const DESCRIPTION: &str =
    "Where your contacts live, what Postio works out about them, and whether they sync.";

pub const STORED_IN: &str = "Stored in Postio's database, not config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionRow {
    pub which: Suggestion,
    pub title: String,
    pub detail: String,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionsView {
    pub heading: String,
    pub rows: Vec<SuggestionRow>,
    pub footer: String,
    pub error: Option<String>,
}

pub fn suggestions(
    switches: &ContactSuggestions,
    device: Device,
    with_sync: bool,
) -> SuggestionsView {
    let rows = Suggestion::ALL
        .into_iter()
        .map(|which| {
            let on = switches.is_on(which);
            let (title, detail) = match which {
                Suggestion::SaveAutomatically => (
                    "Save people I write to, automatically",
                    match (on, with_sync) {
                        (true, true) => "On: people you write to are saved, so they can sync",
                        (true, false) => "On: people you write to are saved without being asked",
                        (false, true) => {
                            "Off: they still get a card, but aren't saved or synced until you add them"
                        }
                        (false, false) => {
                            "Off: they still get a card, but aren't saved until you add them"
                        }
                    },
                ),
                Suggestion::RecogniseKinds => (
                    "Recognise lists, newsletters and automated senders",
                    "From List-Id, List-Unsubscribe, Precedence and no-reply headers",
                ),
                Suggestion::ReadSignatures => (
                    "Read signatures for details",
                    "Suggests phone, title and company; never saved without you",
                ),
                Suggestion::SuggestMerges => (
                    "Suggest merging duplicates",
                    "Same signature, same threads, or forwarding headers",
                ),
            };
            SuggestionRow {
                which,
                title: title.to_owned(),
                detail: detail.to_owned(),
                on,
            }
        })
        .collect();
    let leaves = if with_sync {
        "Nothing leaves it except through the sync accounts above."
    } else {
        "Nothing leaves it."
    };
    SuggestionsView {
        heading: "Suggestions".to_owned(),
        rows,
        footer: format!(
            "Signature reading, kind detection, duplicate checks and stats run on {}. {leaves}",
            device.phrase()
        ),
        error: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intro {
    pub lead: String,
    pub rest: String,
}

/// Says nothing of sync where there is nothing to sync with.
pub fn intro(device: Device, with_sync: bool) -> Intro {
    let sync = if with_sync {
        " Sync is optional and keeps other apps and your phone up to date."
    } else {
        ""
    };
    Intro {
        lead: "Your contacts live in Postio,".to_owned(),
        rest: format!(
            " on {} next to your mail, so every sender has a card with stats from day one, with or without any account.{sync}",
            device.phrase()
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    ReadOnly,
    TwoWay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeOption {
    pub label: String,
    pub mode: ModeChoice,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    NotOn,
    Previewed(String),
    Declined,
    BroughtIn(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSection {
    pub heading: String,
    pub account: AccountRow,
    pub modes: Vec<ModeOption>,
    pub scopes_heading: String,
    pub scopes: Vec<ScopeOption>,
    pub scope_note: String,
    pub preview: String,
    pub plan: Option<PlanView>,
}

pub fn sync_section(
    scope: ScopeChoice,
    mode: ModeChoice,
    status: &SyncStatus,
    plan: Option<PlanView>,
) -> SyncSection {
    let status_line = match status {
        SyncStatus::NotOn => {
            "Not turned on. Nothing is read from Contacts until you do.".to_owned()
        }
        SyncStatus::Previewed(summary) => format!("{summary}. Dry run: nothing was written."),
        SyncStatus::Declined => "Contacts access was declined. Nothing was read.".to_owned(),
        SyncStatus::BroughtIn(done) => done.clone(),
    };
    let note = match mode {
        ModeChoice::ReadOnly => status_line,
        ModeChoice::TwoWay => format!(
            "{status_line} Writing to Contacts is not available yet, so Postio only previews a two-way sync."
        ),
    };
    SyncSection {
        heading: "Sync with".to_owned(),
        account: AccountRow {
            title: "Contacts on this Mac (iCloud)".to_owned(),
            mode: match mode {
                ModeChoice::ReadOnly => "Read only",
                ModeChoice::TwoWay => "Two-way",
            }
            .to_owned(),
            note,
        },
        modes: [
            ("Two-way", ModeChoice::TwoWay),
            ("Read only", ModeChoice::ReadOnly),
        ]
        .into_iter()
        .map(|(label, option)| ModeOption {
            label: label.to_owned(),
            mode: option,
            on: option == mode,
        })
        .collect(),
        scopes_heading: "What syncs".to_owned(),
        scopes: scope_options(scope),
        scope_note: scope_note(scope),
        preview: match status {
            SyncStatus::Previewed(_) | SyncStatus::BroughtIn(_) => "Preview again",
            _ => "Turn on and preview",
        }
        .to_owned(),
        plan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_switches_read_as_screen_08_does_in_their_defaults() {
        let view = suggestions(&ContactSuggestions::default(), Device::Mac, true);
        assert_eq!(view.heading, "Suggestions");
        let titles: Vec<&str> = view.rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Save people I write to, automatically",
                "Recognise lists, newsletters and automated senders",
                "Read signatures for details",
                "Suggest merging duplicates",
            ]
        );
        let on: Vec<bool> = view.rows.iter().map(|row| row.on).collect();
        assert_eq!(on, [false, true, true, true]);
        assert_eq!(
            view.rows[0].detail,
            "Off: they still get a card, but aren't saved or synced until you add them"
        );
        assert_eq!(
            view.footer,
            "Signature reading, kind detection, duplicate checks and stats run on this Mac. Nothing leaves it except through the sync accounts above."
        );
    }

    #[test]
    fn a_pane_without_sync_does_not_mention_sync_accounts_and_says_this_computer() {
        let view = suggestions(&ContactSuggestions::default(), Device::Computer, false);
        assert!(!view.footer.contains("sync"), "{}", view.footer);
        assert!(view.footer.contains("this computer"));
        let words = intro(Device::Computer, false);
        assert!(!words.rest.contains("Mac") && !words.rest.contains("Sync"));
        assert!(intro(Device::Mac, true).rest.contains("Sync is optional"));
        assert_eq!(words.lead, "Your contacts live in Postio,");
    }

    #[test]
    fn the_first_switch_explains_itself_in_both_states() {
        let on = suggestions(
            &ContactSuggestions::default().with(Suggestion::SaveAutomatically, true),
            Device::Mac,
            true,
        );
        assert!(on.rows[0].on && on.rows[0].detail.starts_with("On:"));
    }

    #[test]
    fn the_sync_section_says_how_far_it_got_and_that_two_way_only_previews() {
        let before = sync_section(
            ScopeChoice::PeopleAndCompanies,
            ModeChoice::ReadOnly,
            &SyncStatus::NotOn,
            None,
        );
        assert_eq!(before.heading, "Sync with");
        assert_eq!(before.account.title, "Contacts on this Mac (iCloud)");
        assert_eq!(before.account.mode, "Read only");
        assert!(before.account.note.starts_with("Not turned on"));
        assert_eq!(before.preview, "Turn on and preview");
        assert_eq!(
            before
                .modes
                .iter()
                .map(|m| (m.label.as_str(), m.on))
                .collect::<Vec<_>>(),
            [("Two-way", false), ("Read only", true)]
        );
        assert!(
            before
                .scopes
                .iter()
                .any(|s| s.on && s.label == "People and companies")
        );

        let after = sync_section(
            ScopeChoice::People,
            ModeChoice::TwoWay,
            &SyncStatus::Previewed("Would add 2, update 1, ask about 0, delete 0".to_owned()),
            None,
        );
        assert_eq!(after.account.mode, "Two-way");
        assert!(after.account.note.contains("Would add 2"));
        assert!(after.account.note.contains("only previews a two-way sync"));
        assert_eq!(after.preview, "Preview again");
    }
}
