//! Contacts, built from addresses Postio has seen.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, ContactId};

/// How a contact row first came to exist.
///
/// This is provenance, not a current judgement: it names how the row *first*
/// appeared, and never changes once it does except by the one deliberate
/// promotion `mail` -> `user` (ADR 0007 Q1). A `mail` row the user edits is
/// promoted on the same row, never by inserting a second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactSource {
    /// Accumulated from a message's headers. Never resurrects once
    /// suppressed, and is the only source `delete` merely suppresses rather
    /// than removing (ADR 0007 Q2).
    #[default]
    Mail,
    /// Created or promoted by the user directly.
    User,
    /// Brought in from an imported vCard.
    Import,
}

impl ContactSource {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::User => "user",
            Self::Import => "import",
        }
    }

    /// The inverse of [`ContactSource::as_str`].
    ///
    /// `None` for anything else: a value that is not one of these came from a
    /// corrupt row, and guessing at it would be worse than saying so.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mail" => Some(Self::Mail),
            "user" => Some(Self::User),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// A known correspondent.
///
/// v1 has no address book integration: contacts are accumulated from message
/// headers so recipient autocomplete can rank by how often and how recently an
/// address was seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// Local id.
    pub id: ContactId,
    /// Account this contact was seen through, or `None` when it is shared.
    pub account_id: Option<AccountId>,
    /// A name the user set, overriding whatever headers carried.
    pub name: Option<String>,
    /// The address itself, including the display name last seen on it.
    pub address: EmailAddress,
    /// How many messages this address has been seen on.
    pub times_seen: u32,
    /// When it was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// How this row first came to exist.
    pub source: ContactSource,
    /// Whether this contact is suppressed (ADR 0007 Q2): a deleted `mail`
    /// contact stays a row, but drops out of autocomplete, the `@` finder,
    /// and any contact list.
    pub suppressed: bool,
}

impl Contact {
    /// Builds an unpersisted contact that has not been seen yet.
    pub fn new(address: EmailAddress) -> Self {
        Self {
            id: ContactId::UNASSIGNED,
            account_id: None,
            name: None,
            address,
            times_seen: 0,
            last_seen_at: None,
            source: ContactSource::default(),
            suppressed: false,
        }
    }

    /// Records one more sighting at `at`, advancing `last_seen_at` monotonically.
    pub fn record_seen(&mut self, at: DateTime<Utc>) {
        self.times_seen = self.times_seen.saturating_add(1);
        if self.last_seen_at.is_none_or(|last| at > last) {
            self.last_seen_at = Some(at);
        }
    }

    /// The user-set name, else the last display name seen, else the address.
    pub fn display_name(&self) -> &str {
        match self.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => self.address.display(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_source_round_trips_through_its_stored_identifier() {
        for source in [
            ContactSource::Mail,
            ContactSource::User,
            ContactSource::Import,
        ] {
            assert_eq!(ContactSource::from_name(source.as_str()), Some(source));
        }
        assert_eq!(ContactSource::from_name("scraped"), None);
    }

    #[test]
    fn an_unseen_contact_defaults_to_mail_and_unsuppressed() {
        let contact = Contact::new(EmailAddress::new(Some("Ada"), "ada@example.com"));
        assert_eq!(contact.source, ContactSource::Mail);
        assert!(!contact.suppressed);
    }
}
