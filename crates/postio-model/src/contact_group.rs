//! Contact groups: a named set of people, not a saved search.
//!
//! This is the one place
//! `ARCHITECTURE.md` §6's "one matching language" does not apply: a saved
//! search answers *which messages*; a group answers *which people*, and no
//! query can express "Ada, Grace and Katherine, because I said so"
//! (specs/005-contacts, inherited from the address-book decision's Q3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::ContactGroupId;

/// A named set of people, shared across accounts as people are.
///
/// Expanded into its members' preferred addresses at the moment it is picked
/// in the composer, never referenced by a group address of its own —
/// there is no `family@` to put in a `To:` header, and pretending otherwise
/// would mean a draft whose recipients change between saving and sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroup {
    /// Local id.
    pub id: ContactGroupId,
    /// Display name.
    pub name: String,
    /// vCard `KIND:group` UID, when this group came from or round-trips
    /// through a vCard.
    pub uid: Option<String>,
    /// When the group was created.
    pub created_at: DateTime<Utc>,
}

impl ContactGroup {
    /// Builds an unpersisted group.
    pub fn new(name: impl Into<String>, created_at: DateTime<Utc>) -> Self {
        Self {
            id: ContactGroupId::UNASSIGNED,
            name: name.into(),
            uid: None,
            created_at,
        }
    }
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
