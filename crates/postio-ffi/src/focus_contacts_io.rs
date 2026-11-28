//! Importing and exporting contacts: the Mac presents the open and save panels; parsing, applying,
//! writing and every word are Rust's. An import has no undo and never merges people.

use postio_ui::contacts_io::IoSheetView;

use crate::Session;
use crate::focus_contact_edit::EditButtonFfi;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsPanelWordsFfi {
    pub import_title: String,
    pub import_message: String,
    pub export_title: String,
    pub export_message: String,
    pub export_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsIoSheetFfi {
    /// "Imported 3 contacts".
    pub title: String,
    pub lines: Vec<String>,
    /// Review, when the import left people to look at.
    pub review: Option<EditButtonFfi>,
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
    pub sheet: Option<ContactsIoSheetFfi>,
    /// Why not, in words for the screen.
    pub error: Option<String>,
}

/// An export's answer: the file's text to write where the user chose, and the
/// sheet to show once it is written.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactsExportResultFfi {
    pub text: Option<String>,
    pub sheet: Option<ContactsIoSheetFfi>,
    pub error: Option<String>,
}

#[uniffi::export]
impl Session {
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
            Ok(summary) => {
                // The cursor goes to the first new person.
                self.focus_driver()
                    .input(postio_focus::Input::ContactsImported {
                        made: summary.made.clone(),
                        touched: summary.touched.clone(),
                    });
                ContactsImportResultFfi {
                    sheet: Some(
                        postio_ui::contacts_io::import_sheet(&summary, &self.keymap()).into(),
                    ),
                    error: None,
                }
            }
            Err(error) => ContactsImportResultFfi {
                sheet: None,
                error: Some(error.to_string()),
            },
        }
    }

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
