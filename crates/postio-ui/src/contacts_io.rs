//! Importing and exporting contacts, in words (specs/011-focus-contacts S11):
//! what the open and save panels say, and what the sheet after them says.
//!
//! The file is the user's: it is read or written only through a panel they
//! used, on this machine. An import never merges people behind their back --
//! what looks like someone they have is offered in the review -- and has no
//! undo, which the sheet says plainly. No toolkit here.

use postio_core::{CommandId, Keymap};
use postio_model::card::ImportSummary;

use crate::contact_edit::EditButton;

/// What the panels say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelWords {
    /// The open panel's title and prompt.
    pub import_title: String,
    /// Under it.
    pub import_message: String,
    /// The save panel's title.
    pub export_title: String,
    /// Under it.
    pub export_message: String,
    /// The name it suggests.
    pub export_name: String,
}

/// The panels' words.
pub fn panel_words() -> PanelWords {
    PanelWords {
        import_title: "Import Contacts".to_owned(),
        import_message: "Choose a .vcf file. Postio reads it on this Mac and uploads nothing."
            .to_owned(),
        export_title: "Export Contacts".to_owned(),
        export_message: "Postio writes one .vcf of the people in your Contacts list, to the name and place you choose."
            .to_owned(),
        export_name: "Contacts.vcf".to_owned(),
    }
}

/// The sheet after an import or an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoSheetView {
    /// "Imported 3 contacts".
    pub title: String,
    /// What happened, a line each.
    pub lines: Vec<String>,
    /// "Review", with its key, when the import left people to look at.
    pub review: Option<EditButton>,
    /// "Done", with its key.
    pub done: EditButton,
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The sheet after reading a file: what was made and changed, what is left to
/// review, and that there is no undo.
pub fn import_sheet(summary: &ImportSummary, keymap: &Keymap) -> IoSheetView {
    let touched = summary.people_created + summary.people_updated;
    let title = if touched == 0 && summary.groups_created == 0 {
        "Nothing was imported".to_owned()
    } else {
        format!("Imported {}", count(touched, "contact", "contacts"))
    };
    let mut lines = Vec::new();
    if summary.people_created > 0 {
        lines.push(format!(
            "{} added",
            count(summary.people_created, "new person", "new people")
        ));
    }
    if summary.people_updated > 0 {
        lines.push(format!(
            "{} you already had, matched by address",
            count(summary.people_updated, "person", "people")
        ));
    }
    if summary.groups_created > 0 {
        lines.push(format!(
            "{} made",
            count(summary.groups_created, "group", "groups")
        ));
    }
    if summary.name_conflicts > 0 {
        lines.push(format!(
            "Kept the name you gave for {}",
            count(summary.name_conflicts, "contact", "contacts")
        ));
    }
    for skip in &summary.skipped {
        lines.push(format!("Skipped card {}: {}", skip.index + 1, skip.reason));
    }
    let offered = summary.look_alikes + summary.offered_merges;
    if offered > 0 {
        lines.push(format!(
            "{} may be {} you already have. Nothing was merged: review them to decide.",
            count(offered, "contact", "contacts"),
            if offered == 1 { "someone" } else { "people" }
        ));
    }
    lines.push("An import cannot be undone. Delete a contact to take it back.".to_owned());
    let key = |command| keymap.binding(command).map(str::to_owned);
    IoSheetView {
        title,
        lines,
        review: (offered > 0).then(|| EditButton {
            label: "Review".to_owned(),
            key: key(CommandId::ContactsSuggestions),
        }),
        done: EditButton {
            label: "Done".to_owned(),
            key: key(CommandId::Back),
        },
    }
}

/// The sheet after writing a file.
pub fn export_sheet(count_of: usize, keymap: &Keymap) -> IoSheetView {
    IoSheetView {
        title: format!("Exported {}", count(count_of, "contact", "contacts")),
        lines: vec![
            "Written to the file you named, with their groups.".to_owned(),
            "Nothing else leaves this Mac.".to_owned(),
        ],
        review: None,
        done: EditButton {
            label: "Done".to_owned(),
            key: keymap.binding(CommandId::Back).map(str::to_owned),
        },
    }
}

#[cfg(test)]
mod tests {
    use postio_model::card::Skip;

    use super::*;

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    #[test]
    fn the_panels_say_where_the_file_goes_and_that_nothing_is_uploaded() {
        let words = panel_words();
        assert_eq!(words.import_title, "Import Contacts");
        assert!(words.import_message.contains("uploads nothing"));
        assert_eq!(words.export_name, "Contacts.vcf");
    }

    #[test]
    fn an_import_says_what_was_made_what_matched_and_what_is_left_to_review() {
        let sheet = import_sheet(
            &ImportSummary {
                people_created: 2,
                people_updated: 1,
                groups_created: 1,
                name_conflicts: 1,
                skipped: vec![Skip {
                    index: 3,
                    reason: "it has no email address".into(),
                }],
                offered_merges: 0,
                look_alikes: 1,
            },
            &keymap(),
        );
        assert_eq!(sheet.title, "Imported 3 contacts");
        assert_eq!(
            sheet.lines,
            [
                "2 new people added",
                "1 person you already had, matched by address",
                "1 group made",
                "Kept the name you gave for 1 contact",
                "Skipped card 4: it has no email address",
                "1 contact may be someone you already have. Nothing was merged: review them to decide.",
                "An import cannot be undone. Delete a contact to take it back."
            ]
        );
        let review = sheet.review.expect("something to review");
        assert_eq!(
            (review.label.as_str(), review.key.as_deref()),
            ("Review", Some("g w"))
        );
        assert_eq!(sheet.done.key.as_deref(), Some("Escape"));
    }

    #[test]
    fn an_import_with_nothing_to_review_offers_no_review_and_an_empty_one_says_so() {
        let plain = import_sheet(
            &ImportSummary {
                people_created: 1,
                ..ImportSummary::default()
            },
            &keymap(),
        );
        assert!(plain.review.is_none());
        assert_eq!(plain.title, "Imported 1 contact");
        let empty = import_sheet(&ImportSummary::default(), &keymap());
        assert_eq!(empty.title, "Nothing was imported");
    }

    #[test]
    fn an_export_says_how_many_and_that_nothing_else_left() {
        let sheet = export_sheet(5, &keymap());
        assert_eq!(sheet.title, "Exported 5 contacts");
        assert!(
            sheet
                .lines
                .iter()
                .any(|l| l == "Nothing else leaves this Mac.")
        );
        assert_eq!(export_sheet(1, &keymap()).title, "Exported 1 contact");
    }
}
