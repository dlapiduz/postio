//! A vCard as Postio reads it: the shapes `postio-vcard` parses into and the
//! store applies (specs/005-contacts User Story 6, contracts/vcard.md).
//!
//! Here rather than in `postio-vcard` so the store can apply an import without
//! depending on the parser: the crate that reads files and the crate that
//! writes rows meet on data, not on each other.

use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;

/// What a file held: the cards that read, and the ones that did not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// Every card that could be read, in file order.
    pub cards: Vec<ParsedCard>,
    /// Every card that could not, with why.
    pub skipped: Vec<Skip>,
}

/// A card that was not imported (FR-054).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    /// Its position in the file, from zero.
    pub index: usize,
    /// Why, for the summary.
    pub reason: String,
}

/// One card, as Postio reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParsedCard {
    /// A person.
    Person(ParsedPerson),
    /// A `KIND:group` card.
    Group(ParsedGroup),
}

/// A person's card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPerson {
    /// `UID`.
    pub uid: Option<String>,
    /// `FN`.
    pub name: Option<String>,
    /// Every `EMAIL`, and whether it is the preferred one; exactly one is.
    pub emails: Vec<(EmailAddress, bool)>,
    /// `ORG`.
    pub organization: Option<String>,
    /// `NOTE`.
    pub note: Option<String>,
    /// The card as it arrived, for `contacts.vcard`.
    pub raw: String,
}

/// A group card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedGroup {
    /// `UID`.
    pub uid: Option<String>,
    /// `FN`.
    pub name: String,
    /// Its `MEMBER`s.
    pub members: Vec<Member>,
    /// The card as it arrived.
    pub raw: String,
}

/// A `MEMBER` of a group card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Member {
    /// Another card, by its `UID`.
    Uid(String),
    /// An address, from a `mailto:` URI.
    Address(String),
}

/// What applying an import did, for the summary the user sees (FR-053,
/// R9). Logged as counts only (FR-061).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    /// People who were not in the address book before.
    pub people_created: usize,
    /// People a card was matched to by address.
    pub people_updated: usize,
    /// People a card's addresses spanned, joined into one: their names.
    pub joins: Vec<Vec<String>>,
    /// Groups made.
    pub groups_created: usize,
    /// Cards whose name lost to one the user had set.
    pub name_conflicts: usize,
    /// Cards not imported, with why.
    pub skipped: Vec<Skip>,
}
