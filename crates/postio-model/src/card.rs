//! A vCard as Postio reads it. Lives here so the store applies an import without depending on the
//! parser.

use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub cards: Vec<ParsedCard>,
    pub skipped: Vec<Skip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    pub index: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParsedCard {
    Person(ParsedPerson),
    Group(ParsedGroup),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPerson {
    pub uid: Option<String>,
    pub name: Option<String>,
    /// Every `EMAIL`, and whether it is the preferred one; exactly one is.
    pub emails: Vec<(EmailAddress, bool)>,
    pub organization: Option<String>,
    pub title: Option<String>,
    pub phones: Vec<String>,
    pub note: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedGroup {
    pub uid: Option<String>,
    pub name: String,
    pub members: Vec<Member>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Member {
    Uid(String),
    Address(String),
}

/// Logged as counts only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    pub people_created: usize,
    pub people_updated: usize,
    /// Cards that listed the addresses of more than one person you have:
    /// offered as a merge in the review, never joined by the import.
    pub offered_merges: usize,
    /// Offered as a merge, never a silent duplicate.
    pub look_alikes: usize,
    pub made: Vec<crate::ids::ContactId>,
    pub touched: Vec<crate::ids::ContactId>,
    pub groups_created: usize,
    pub name_conflicts: usize,
    pub skipped: Vec<Skip>,
}
