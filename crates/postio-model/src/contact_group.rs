//! Contact groups: a named set of people, not a saved search.
//!
//! ADR 0007 Q3 is explicit that this is the one place
//! `ARCHITECTURE.md` §6's "one matching language" does not apply: a saved
//! search answers *which messages*; a group answers *which people*, and no
//! query can express "Ada, Grace and Katherine, because I said so".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, ContactGroupId};

/// A named set of contacts.
///
/// Expanded into its members' addresses at the moment it is picked in the
/// composer, never referenced by a group address of its own (ADR 0007 Q3) —
/// there is no `family@` to put in a `To:` header, and pretending otherwise
/// would mean a draft whose recipients change between saving and sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroup {
    /// Local id.
    pub id: ContactGroupId,
    /// Account this group belongs to, or `None` when it is shared —
    /// matching `Contact::account_id`, and defaulting the same way (ADR
    /// 0007 Q5): a group the user creates is shared unless they say
    /// otherwise.
    pub account_id: Option<AccountId>,
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
    pub fn new(
        account_id: Option<AccountId>,
        name: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: ContactGroupId::UNASSIGNED,
            account_id,
            name: name.into(),
            uid: None,
            created_at,
        }
    }
}
