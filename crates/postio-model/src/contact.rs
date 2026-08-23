//! Contacts, built from addresses Postio has seen.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, ContactId};

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
