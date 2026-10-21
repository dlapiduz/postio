//! Importing and exporting contacts across the boundary (specs/011-focus-contacts
//! S11). The Mac presents the open and save panels (the file is the user's,
//! through a panel they used); parse, apply, write-out and every word are
//! Rust's. An import has no undo, and never merges people behind the
//! user's back.

use postio_ui::contacts_io::IoSheetView;

use crate::Session;
use crate::focus_contact_edit::EditButtonFfi;

/// What the panels say.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsPanelWordsFfi {
    /// The open panel's title.
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

/// The sheet after an import or an export.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsIoSheetFfi {
    /// "Imported 3 contacts".
    pub title: String,
    /// What happened.
    pub lines: Vec<String>,
    /// Review, when the import left people to look at.
    pub review: Option<EditButtonFfi>,
    /// Done.
    pub done: EditButtonFfi,
}

impl From<IoSheetView> for ContactsIoSheetFfi {
    fn from(sheet: IoSheetView) -> Self {
        let button = |button: postio_ui::contact_edit::EditButton| EditButtonFfi {
            label: button.label,
            key: button.key,
        };
        ContactsIoSheetFfi {
            title: sheet.title,
            lines: sheet.lines,
            review: sheet.review.map(button),
            done: button(sheet.done),
        }
    }
}

/// An import's answer: the sheet to show, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsImportResultFfi {
    /// What was done.
    pub sheet: Option<ContactsIoSheetFfi>,
    /// Why not, in words for the screen.
    pub error: Option<String>,
}

/// An export's answer: the file's text to write where the user chose, and the
/// sheet to show once it is written.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsExportResultFfi {
    /// The `.vcf`.
    pub text: Option<String>,
    /// The sheet.
    pub sheet: Option<ContactsIoSheetFfi>,
    /// Why not.
    pub error: Option<String>,
}

#[uniffi::export]
impl Session {
    /// What the open and save panels say.
    pub fn contacts_panel_words(&self) -> ContactsPanelWordsFfi {
        let words = postio_ui::contacts_io::panel_words();
        ContactsPanelWordsFfi {
            import_title: words.import_title,
            import_message: words.import_message,
            export_title: words.export_title,
            export_message: words.export_message,
            export_name: words.export_name,
        }
    }

    /// Reads the cards in `bytes`, a file the user chose in an open panel,
    /// and applies them. Nothing is merged; there is no undo.
    pub fn contacts_import(&self, bytes: Vec<u8>) -> ContactsImportResultFfi {
        let Some(client) = self.client() else {
            return ContactsImportResultFfi {
                sheet: None,
                error: Some("Postio is still starting".to_owned()),
            };
        };
        match crate::session::blocking(client.import_contacts(bytes)) {
            Ok(summary) => ContactsImportResultFfi {
                sheet: Some(postio_ui::contacts_io::import_sheet(&summary, &self.keymap()).into()),
                error: None,
            },
            Err(error) => ContactsImportResultFfi {
                sheet: None,
                error: Some(error.to_string()),
            },
        }
    }

    /// One `.vcf` of the people the Contacts list holds, to write where the
    /// user chose in a save panel.
    pub fn contacts_export(&self) -> ContactsExportResultFfi {
        let Some(client) = self.client() else {
            return ContactsExportResultFfi {
                text: None,
                sheet: None,
                error: Some("Postio is still starting".to_owned()),
            };
        };
        match crate::session::blocking(client.export_contacts()) {
            Ok((text, count)) => ContactsExportResultFfi {
                text: Some(text),
                sheet: Some(postio_ui::contacts_io::export_sheet(count, &self.keymap()).into()),
                error: None,
            },
            Err(error) => ContactsExportResultFfi {
                text: None,
                sheet: None,
                error: Some(error.to_string()),
            },
        }
    }
}
