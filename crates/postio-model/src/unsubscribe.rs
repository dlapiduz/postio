//! Append-only record of one-click-unsubscribe activations (#971), so the privacy pane can show
//! when unsubscribing happened.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, UnsubscribeActivationId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsubscribeActivation {
    pub id: UnsubscribeActivationId,
    pub account_id: AccountId,
    /// From `List-Id` when present, else the sender's domain: whichever the reader found to
    /// activate against.
    pub list_identifier: String,
    pub activated_at: DateTime<Utc>,
}

impl UnsubscribeActivation {
    /// `activated_at` is the caller's to supply, so a test controls what it asserts on.
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
