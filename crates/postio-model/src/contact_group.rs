//! A named set of people, not a saved search: no query can express a hand-picked set of people.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::ContactGroupId;

/// Expanded into its members' preferred addresses when picked in the composer, never referenced by
/// a group
/// address: a draft's recipients must not change between saving and sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroup {
    pub id: ContactGroupId,
    pub name: String,
    pub uid: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ContactGroup {
    pub fn new(name: impl Into<String>, created_at: DateTime<Utc>) -> Self {
        Self {
            id: ContactGroupId::UNASSIGNED,
            name: name.into(),
            uid: None,
            created_at,
        }
    }
}

/// Accepting a group inserts every member individually, never a group reference: recipients must
/// stay visible
/// (keeps `Bcc` honest) and cannot change when membership is edited after picking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecipientCandidate {
    Contact(EmailAddress),
    /// `members` is the membership when the candidate was offered.
    Group {
        name: String,
        members: Vec<EmailAddress>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_group_carries_no_vcard_link_until_one_is_given() {
        let group = ContactGroup::new("Book club", Utc::now());
        assert_eq!(group.uid, None);
    }
}
