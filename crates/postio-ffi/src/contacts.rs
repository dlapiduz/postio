//! Recipient completion for the Mac's composer (specs/009-focus-macos
//! T075): `postio_ui::recipients::suggest` -- the rule GTK's composer and
//! the terminal rank by -- over the account's directory, with the people
//! the Mac's Contacts lends for the keystroke beside it.
//!
//! The directory is read through the host and kept for a minute, as GTK's
//! composer keeps it from one opening to the next, so a keystroke runs no
//! query. What Contacts lends is used for the one answer and kept nowhere:
//! it never reaches the store or a log.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use postio_client::protocol::RecipientDirectory;
use postio_model::contact_group::RecipientCandidate;
use postio_model::{AccountId, Contact, ContactSource, EmailAddress};
use postio_ui::recipients::Correspondent;

use crate::Session;

/// How long a directory is kept before it is read again: a contact first
/// seen in mail that arrived meanwhile is offered within a minute.
const DIRECTORY_FOR: Duration = Duration::from_secs(60);

/// Each account's directory, as last read.
pub(crate) type Directories = Mutex<HashMap<AccountId, (Instant, RecipientDirectory)>>;

/// Someone from the platform's address book, offered for this keystroke.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExternalContactFfi {
    /// Their name, as the address book has it.
    pub name: Option<String>,
    /// Their address.
    pub address: String,
}

/// One suggestion for the recipient being typed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientSuggestionFfi {
    /// What the row says: an address, or a group's name and size.
    pub label: String,
    /// The field's whole text once this is accepted: the entry being typed
    /// replaced, a group's members each as their own address.
    pub accepted: String,
    /// Whether it is a group, which inserts several addresses.
    pub group: bool,
}

#[uniffi::export]
impl Session {
    /// At most `limit` suggestions for the recipient being typed at the end
    /// of `text` in `account`'s composer, or none until enough of it is
    /// typed. `extra` is what the platform's address book lends for this
    /// keystroke; an address the directory has is offered once.
    pub fn recipient_suggestions(
        &self,
        account: i64,
        text: String,
        limit: u32,
        extra: Vec<ExternalContactFfi>,
    ) -> Vec<RecipientSuggestionFfi> {
        let Some(prefix) = postio_ui::recipients::prefix(&text) else {
            return Vec::new();
        };
        let directory = self.recipient_directory(AccountId::new(account));
        let known: HashSet<String> = directory
            .contacts
            .iter()
            .map(|row| row.contact.address.normalized())
            .collect();
        let mut contacts = directory.contacts.clone();
        contacts.extend(
            extra
                .into_iter()
                .filter(|lent| !lent.address.trim().is_empty())
                .map(|lent| EmailAddress::new(lent.name, lent.address.trim()))
                .filter(|address| !known.contains(&address.normalized()))
                .map(|address| {
                    let mut contact = Contact::new(address);
                    contact.source = ContactSource::Import;
                    Correspondent::from(contact)
                }),
        );
        postio_ui::recipients::suggest(&directory.groups, &contacts, prefix, limit as usize)
            .into_iter()
            .map(|candidate| RecipientSuggestionFfi {
                label: postio_ui::recipients::candidate_label(&candidate),
                accepted: postio_ui::recipients::accepted(&text, &candidate),
                group: matches!(candidate, RecipientCandidate::Group { .. }),
            })
            .collect()
    }
}

impl Session {
    /// `account`'s directory: kept, or read through the host when it is
    /// older than [`DIRECTORY_FOR`]. Empty while there is no host.
    fn recipient_directory(&self, account: AccountId) -> RecipientDirectory {
        if let Some((read, directory)) = self
            .recipient_directories
            .lock()
            .expect("directory lock")
            .get(&account)
            && read.elapsed() < DIRECTORY_FOR
        {
            return directory.clone();
        }
        let Some(client) = self.client() else {
            return RecipientDirectory::default();
        };
        match crate::session::blocking(client.recipient_directory(account)) {
            Ok(directory) => {
                self.recipient_directories
                    .lock()
                    .expect("directory lock")
                    .insert(account, (Instant::now(), directory.clone()));
                directory
            }
            Err(error) => {
                tracing::warn!(%error, "could not read the recipient directory");
                RecipientDirectory::default()
            }
        }
    }
}
