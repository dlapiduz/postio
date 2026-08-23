//! User labels.

use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, LabelId};

/// A user-visible label that can be applied to many messages.
///
/// Labels are Postio's own concept: on IMAP they are backed by keywords, on
/// providers that have real labels they map onto those.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// Local id.
    pub id: LabelId,
    /// Owning account.
    pub account_id: AccountId,
    /// Display name; unique per account, case-insensitively.
    pub name: String,
    /// Optional hex colour, e.g. `#5980a6`.
    pub color: Option<String>,
}

impl Label {
    /// Builds an unpersisted label.
    pub fn new(account_id: AccountId, name: impl Into<String>) -> Self {
        Self {
            id: LabelId::UNASSIGNED,
            account_id,
            name: name.into(),
            color: None,
        }
    }
}
