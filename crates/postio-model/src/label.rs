use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, LabelId};

/// Backed by keywords on IMAP, mapped onto real labels where the provider has them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: LabelId,
    pub account_id: AccountId,
    /// Display name; unique per account, case-insensitively.
    pub name: String,
    pub color: Option<String>,
}

impl Label {
    pub fn new(account_id: AccountId, name: impl Into<String>) -> Self {
        Self {
            id: LabelId::UNASSIGNED,
            account_id,
            name: name.into(),
            color: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_label_starts_uncoloured() {
        let label = Label::new(AccountId::new(1), "Receipts");
        assert_eq!(label.color, None);
    }
}
