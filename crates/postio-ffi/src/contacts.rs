//! Recipient completion for the Mac's composer: `postio_ui::recipients::suggest_ranked` over the
//! account's directory. The directory is kept for a minute so a keystroke runs no query; what
//! Contacts lends is used for one answer and never stored or logged.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use postio_client::protocol::RecipientDirectory;
use postio_model::contact_group::RecipientCandidate;
use postio_model::{AccountId, AddressId, Contact, ContactAddress, ContactSource, EmailAddress};
use postio_ui::recipients::Correspondent;

use crate::Session;

/// How long a directory is kept before it is read again.
const DIRECTORY_FOR: Duration = Duration::from_secs(60);

pub(crate) type Directories = Mutex<HashMap<AccountId, (Instant, RecipientDirectory)>>;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExternalContactFfi {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactKindFfi {
    Person,
    Company,
    List,
    Newsletter,
    Automated,
}

impl From<postio_model::ContactKind> for ContactKindFfi {
    fn from(kind: postio_model::ContactKind) -> Self {
        match kind {
            postio_model::ContactKind::Person => Self::Person,
            postio_model::ContactKind::Company => Self::Company,
            postio_model::ContactKind::List => Self::List,
            postio_model::ContactKind::Newsletter => Self::Newsletter,
            postio_model::ContactKind::Automated => Self::Automated,
        }
    }
}

impl From<ContactKindFfi> for postio_model::ContactKind {
    fn from(kind: ContactKindFfi) -> Self {
        match kind {
            ContactKindFfi::Person => Self::Person,
            ContactKindFfi::Company => Self::Company,
            ContactKindFfi::List => Self::List,
            ContactKindFfi::Newsletter => Self::Newsletter,
            ContactKindFfi::Automated => Self::Automated,
        }
    }
}

/// A person, as the card and the popover draw them; every word is made in Rust
/// (`Contact::shown_as`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactFfi {
    pub id: i64,
    /// The name in the style the person asked for, else what the mail called
    /// them, else the preferred address.
    pub name: String,
    pub kind: ContactKindFfi,
    pub saved: bool,
    /// Every address they own, the preferred one first.
    pub addresses: Vec<String>,
    pub times_seen: u32,
    pub written: u32,
}

impl From<&Contact> for ContactFfi {
    fn from(person: &Contact) -> Self {
        Self {
            id: person.id.get(),
            name: person.shown_as().into_owned(),
            kind: person.kind.into(),
            saved: person.is_saved(),
            addresses: person
                .addresses
                .iter()
                .map(|a| a.address.address.clone())
                .collect(),
            times_seen: person.times_seen,
            written: person.written,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientSuggestionFfi {
    pub label: String,
    /// The field's whole text once this is accepted: the entry being typed
    /// replaced, a group's members each as their own address.
    pub accepted: String,
    pub group: bool,
}

#[uniffi::export]
impl Session {
    /// Made now from stored mail if the background catch-up has not reached them. `None` for an
    /// address the store never saw, the user's own, or while there is no host.
    pub fn contact_for_address(&self, address: String) -> Option<ContactFfi> {
        let client = self.client()?;
        match crate::session::blocking(client.contact_for_address(&address)) {
            Ok(person) => person.as_ref().map(ContactFfi::from),
            Err(error) => {
                tracing::warn!(%error, "could not read the person at an address");
                None
            }
        }
    }

    /// `extra` is what the platform's address book lends for this keystroke; an address the
    /// directory has is offered once. None until enough is typed.
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
            .flat_map(|row| &row.contact.addresses)
            .map(|owned| owned.address.normalized())
            .collect();
        let mut contacts = directory.contacts.clone();
        contacts.extend(
            extra
                .into_iter()
                .filter(|lent| !lent.address.trim().is_empty())
                .map(|lent| EmailAddress::new(lent.name, lent.address.trim()))
                .filter(|address| !known.contains(&address.normalized()))
                .map(|address| {
                    // Lent for one keystroke and never stored, so the id is nobody's.
                    let mut contact =
                        Contact::new(ContactAddress::new(AddressId::UNASSIGNED, address));
                    contact.source = ContactSource::Import;
                    Correspondent::from(contact)
                }),
        );
        postio_ui::recipients::suggest_ranked(
            &directory.groups,
            &contacts,
            prefix,
            limit as usize,
            true,
        )
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
