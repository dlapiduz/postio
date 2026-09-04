//! The one-click-unsubscribe activation log (#971).
//!
//! CLAUDE.md's privacy section is explicit that unsubscribing happens "only
//! on deliberate activation" — this is the record of when that happened,
//! append-only, so the privacy settings pane can show it back rather than
//! the activation being a silent fact only the reader ever saw. This issue
//! only *logs* the activation; whether pressing the button also sends the
//! real RFC 8058 request is #972, split out on purpose (see that issue).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, UnsubscribeActivationId};

/// One activation: a sender's list, and when the user asked to leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsubscribeActivation {
    /// Local id.
    pub id: UnsubscribeActivationId,
    /// Which account's mail this activation came from.
    pub account_id: AccountId,
    /// The mailing list, from the message's `List-Id` header when it has
    /// one, or the sender's domain otherwise — whichever the reader found
    /// to activate against, not re-derived here.
    pub list_identifier: String,
    /// When the user activated it.
    pub activated_at: DateTime<Utc>,
}

impl UnsubscribeActivation {
    /// Builds an unpersisted activation. `activated_at` is the caller's to
    /// supply — matching `ContactGroup::new`'s own `created_at` parameter —
    /// rather than read here, so a test controls what it asserts on.
    pub fn new(
        account_id: AccountId,
        list_identifier: impl Into<String>,
        activated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: UnsubscribeActivationId::UNASSIGNED,
            account_id,
            list_identifier: list_identifier.into(),
            activated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_activation_carries_no_id_until_persisted() {
        let activation =
            UnsubscribeActivation::new(AccountId::new(1), "newsletter.example.com", Utc::now());
        assert_eq!(activation.id, UnsubscribeActivationId::UNASSIGNED);
        assert_eq!(activation.list_identifier, "newsletter.example.com");
    }
}
