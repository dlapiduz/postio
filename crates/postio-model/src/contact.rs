//! Contacts: the people the user corresponds with, and the addresses they own.
//!
//! A contact is a *person*, not an address (specs/005-contacts). A person owns
//! one or more addresses, and an address belongs to at most one person. What
//! the mail tells us — how often and how recently an address was seen — is a
//! fact about the address and stays with it; the name the user gives, the
//! organisation and the note belong to the person. Every address seen in mail
//! starts as a person of its own, and the user joins them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AddressId, ContactId};

/// How a person first came to exist.
///
/// This is provenance, not a current judgement: it names how the person
/// *first* appeared, and never changes once it does except by the one
/// deliberate promotion `mail` -> `user`. A person known only from mail whom
/// the user edits or joins is promoted in place, never by inserting a second
/// one (specs/005-contacts FR-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactSource {
    /// Accumulated from a message's headers.
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

/// Whether a person is shown, hidden, or folded into another.
///
/// One mechanism serves deletion, restore and exact undo (specs/005-contacts
/// research R4). A deleted person keeps its addresses, so mail from them keeps
/// counting toward someone hidden — which is what stops the next message from
/// bringing them back — and restoring returns them whole. A person absorbed by
/// a join is `Merged`: it keeps its fields and memberships so the join can be
/// undone exactly, and is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContactState {
    /// Shown, offered in completion, and used for names in the mail.
    #[default]
    Live,
    /// Deleted by the user: offered nowhere, restorable.
    Deleted,
    /// Absorbed by a join into another person.
    Merged,
}

impl ContactState {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Deleted => "deleted",
            Self::Merged => "merged",
        }
    }

    /// The inverse of [`ContactState::as_str`]; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "live" => Some(Self::Live),
            "deleted" => Some(Self::Deleted),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }
}

/// One address a person owns, with what the mail says about it.
///
/// The counts are summed across the accounts the address was seen through;
/// the store keeps them per account as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactAddress {
    /// The store's id for this address.
    pub id: AddressId,
    /// The address, carrying the display name the mail most recently gave it.
    pub address: EmailAddress,
    /// How many messages this address has been seen on.
    pub times_seen: u32,
    /// When it was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// How many messages the user sent to it.
    pub written: u32,
}

impl ContactAddress {
    /// An address with no sightings yet.
    pub fn new(id: AddressId, address: EmailAddress) -> Self {
        Self {
            id,
            address,
            times_seen: 0,
            last_seen_at: None,
            written: 0,
        }
    }

    /// Records one more sighting at `at`, advancing `last_seen_at` monotonically.
    pub fn record_seen(&mut self, at: DateTime<Utc>) {
        self.times_seen = self.times_seen.saturating_add(1);
        if self.last_seen_at.is_none_or(|last| at > last) {
            self.last_seen_at = Some(at);
        }
    }
}

/// A person the user corresponds with.
///
/// Built from the mail with no setup, and edited, joined and deleted by the
/// user. The counts at this level are aggregates over the person's addresses,
/// kept so the list can order and filter people without adding them up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// Local id.
    pub id: ContactId,
    /// The name the user set or picked on a join. Never written by sync.
    pub name: Option<String>,
    /// Organisation, as the user entered it.
    pub organization: Option<String>,
    /// A free-text note.
    pub note: Option<String>,
    /// How this person first came to exist.
    pub source: ContactSource,
    /// Shown, deleted, or folded into another person.
    pub state: ContactState,
    /// The address offered first in completion and used by "write to". One of
    /// [`Contact::addresses`]; a stale id falls back to the first.
    pub preferred: AddressId,
    /// Every address this person owns.
    pub addresses: Vec<ContactAddress>,
    /// The display name most recently seen on any of the person's addresses.
    pub seen_name: Option<String>,
    /// Messages any of the person's addresses has been seen on.
    pub times_seen: u32,
    /// When any of them was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Messages the user sent to any of them. More than zero puts the person
    /// in the Contacts list's default view.
    pub written: u32,
}

impl Contact {
    /// Builds an unpersisted person owning one address.
    pub fn new(address: ContactAddress) -> Self {
        Self {
            id: ContactId::UNASSIGNED,
            name: None,
            organization: None,
            note: None,
            source: ContactSource::default(),
            state: ContactState::default(),
            preferred: address.id,
            seen_name: address.address.name.clone(),
            times_seen: address.times_seen,
            last_seen_at: address.last_seen_at,
            written: address.written,
            addresses: vec![address],
        }
    }

    /// The preferred address, or the first one when the preferred id is not
    /// among the person's own.
    pub fn preferred_address(&self) -> Option<&ContactAddress> {
        self.addresses
            .iter()
            .find(|a| a.id == self.preferred)
            .or_else(|| self.addresses.first())
    }

    /// The user-set name, else the display name last seen, else the
    /// preferred address.
    pub fn display_name(&self) -> &str {
        fn filled(value: Option<&str>) -> Option<&str> {
            value.filter(|v| !v.trim().is_empty())
        }
        filled(self.name.as_deref())
            .or_else(|| filled(self.seen_name.as_deref()))
            .or_else(|| self.preferred_address().map(|a| a.address.address.as_str()))
            .unwrap_or("")
    }
}

/// Which people the Contacts list shows (specs/005-contacts FR-005, FR-023a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ContactView {
    /// People the user made or imported, and people the user has written to.
    /// A sender the user only ever received mail from is not here, though it
    /// still completes in the composer.
    #[default]
    Written,
    /// Every live person, however they came to be known.
    Everyone,
    /// People the user deleted, for restoring.
    Deleted,
}

/// One row of the Contacts list: what the row draws, and where it sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactListRow {
    /// The person.
    pub id: ContactId,
    /// Their displayed name ([`Contact::display_name`]).
    pub name: String,
    /// Their preferred address, if they have one.
    pub preferred: Option<String>,
    /// How many addresses they own.
    pub address_count: u32,
    /// When any of their addresses was last seen in mail.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// How they came to exist, so the list can mark the ones the user made.
    pub source: ContactSource,
    /// Their state; only the Deleted view shows anything but `Live`.
    pub state: ContactState,
    /// The list's ordering key, which the next page seeks past.
    pub sort_key: String,
}

/// Everything the Contacts detail view shows about one person (FR-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDetail {
    /// The person, with every address and what the mail says about each.
    pub person: Contact,
    /// The names of the groups they belong to.
    pub groups: Vec<String>,
    /// How many distinct messages involve any of their addresses.
    pub messages: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(id: i64, name: Option<&str>, addr: &str) -> ContactAddress {
        ContactAddress::new(AddressId::new(id), EmailAddress::new(name, addr))
    }

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
    fn contact_state_round_trips_and_refuses_what_it_does_not_know() {
        for state in [
            ContactState::Live,
            ContactState::Deleted,
            ContactState::Merged,
        ] {
            assert_eq!(ContactState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(ContactState::from_name("archived"), None);
        assert_eq!(ContactState::from_name(""), None);
    }

    #[test]
    fn a_new_person_is_live_from_mail_and_owns_the_one_address_it_was_built_from() {
        let contact = Contact::new(address(4, Some("Ada"), "ada@example.com"));
        assert_eq!(contact.source, ContactSource::Mail);
        assert_eq!(contact.state, ContactState::Live);
        assert_eq!(contact.addresses.len(), 1);
        assert_eq!(contact.preferred, AddressId::new(4));
        assert_eq!(
            contact
                .preferred_address()
                .map(|a| a.address.address.as_str()),
            Some("ada@example.com")
        );
    }

    #[test]
    fn the_displayed_name_is_the_users_then_the_mails_then_the_preferred_address() {
        let mut contact = Contact::new(address(1, None, "ada@work.example"));
        contact.addresses.push(address(2, None, "ada@home.example"));
        contact.preferred = AddressId::new(2);
        assert_eq!(contact.display_name(), "ada@home.example");

        contact.seen_name = Some("A. Lovelace".into());
        assert_eq!(contact.display_name(), "A. Lovelace");

        contact.name = Some("Ada Lovelace".into());
        assert_eq!(contact.display_name(), "Ada Lovelace");
    }

    #[test]
    fn a_blank_name_is_no_name_at_either_level() {
        let mut contact = Contact::new(address(1, None, "ada@example.com"));
        contact.name = Some("   ".into());
        contact.seen_name = Some("\t".into());
        assert_eq!(contact.display_name(), "ada@example.com");
    }

    #[test]
    fn a_preferred_id_that_is_not_one_of_the_persons_own_falls_back_to_the_first() {
        let mut contact = Contact::new(address(1, None, "ada@example.com"));
        contact.preferred = AddressId::new(99);
        assert_eq!(contact.display_name(), "ada@example.com");
    }

    #[test]
    fn an_address_counts_its_sightings_and_keeps_the_latest_date() {
        let mut addr = address(1, Some("Ada"), "ada@example.com");
        let later = DateTime::from_timestamp(20, 0).unwrap();
        let earlier = DateTime::from_timestamp(10, 0).unwrap();
        addr.record_seen(later);
        addr.record_seen(earlier);
        assert_eq!(addr.times_seen, 2);
        assert_eq!(addr.last_seen_at, Some(later));
    }
}
