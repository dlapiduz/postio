//! One-click unsubscribe as the boundary carries it (#971): what a message offers, and what was
//! already left.
//! The offer carries no URL or handle, so rendering a message cannot reach the action; the
//! activation re-reads the message.

/// `None` for ordinary personal mail and anything outgoing, or the reader would offer to
/// unsubscribe its owner from their own domain (#1525).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeOfferFfi {
    /// The `List-Id` header when present, else the sender's domain. Crosses for display and tests,
    /// not to be handed back.
    pub list_identifier: String,
    pub summary: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListMailOfferFfi {
    pub summary: String,
    pub count: u32,
    pub button: String,
    pub delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeActivationFfi {
    pub list_identifier: String,
    pub when: String,
    pub label: String,
}
