//! One-click unsubscribe, as the boundary carries it (#971).
//!
//! Two records and nothing else: **what a message offers** and **what was
//! already left**. The verb itself is a method on the session
//! ([`Session::activate_unsubscribe`](crate::Session::activate_unsubscribe)),
//! deliberately not a field here, because the whole privacy claim rests on
//! where it can be called from.
//!
//! # Why the offer carries no way to act
//!
//! CLAUDE.md's privacy section allows one-click unsubscribe "only on
//! deliberate activation", and the thing that makes that structural rather
//! than aspirational is that *rendering a message cannot reach the action*.
//! A `UnsubscribeOfferFfi` is a sentence and an identifier — no URL, no
//! handle, no closure — so a frontend that draws it has drawn a label. The
//! only path to the activation is a method call the frontend makes from a
//! button, with a message id it already had.
//!
//! The identifier is not a target either. It is a `List-Id` or a domain, from
//! the store's own row; nothing the sender wrote decides what a later request
//! would go to, because the activation re-reads the message rather than
//! taking anything from this record back.

/// What the reading pane offers about a message that came from a list.
///
/// `None` from [`Session::unsubscribe_offer`](crate::Session::unsubscribe_offer)
/// for ordinary personal mail and for anything outgoing — the reader's own
/// mail would otherwise offer to unsubscribe its owner from their own domain
/// (#1525).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeOfferFfi {
    /// The list, as the activation log will record it: the `List-Id` header
    /// when the message carries one, the sender's domain otherwise.
    ///
    /// Crosses so the banner can say it and a test can assert on it — not so
    /// a frontend can hand it back. See the module documentation.
    pub list_identifier: String,
    /// The banner's sentence: `This message is from news.example.org`.
    pub summary: String,
    /// What the button is labelled — `postio_ui::unsubscribe::ACTION`, so the
    /// two frontends spell the verb the same way.
    pub action: String,
}

/// One past activation, for the Privacy pane.
///
/// The same argument the remote-image grants make: an action the user cannot
/// see afterwards is one they cannot audit, and "only on deliberate
/// activation" only means something if the deliberate ones are reviewable.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeActivationFfi {
    /// The list that was left.
    pub list_identifier: String,
    /// The date it happened, as the pane's second column: `2026-09-21`.
    pub when: String,
    /// The row as one sentence, for a screen reader that would otherwise
    /// announce two unrelated columns.
    pub label: String,
}
