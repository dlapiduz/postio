//! The classifier's answer: a fixed schema with no text of its own.
//!
//! FR-132 and ADR 0009 ask that whatever classifies can produce a category,
//! spans into the message, and dates -- and never words of its own, because
//! a model's words are attacker-shaped text by the time they reach a row.
//! So nothing here is a `String` anybody outside this crate can fill:
//!
//! - a marker's quote is a [`Span`] of the message's own text, so it is
//!   verbatim by construction, and the caller cuts the excerpt from the text;
//! - the few names an outcome carries -- a filter's source, a digest rule, an
//!   invitation's UID -- are newtypes this crate alone constructs, from the
//!   senders table, the user's config and the calendar part. A model layer in
//!   another crate cannot make one.

use std::ops::Range;

use chrono::{DateTime, Utc};

/// One message's classification: what the filing pass and the body stage
/// act on (`specs/007-postio-focus` data-model, "Classification output").
///
/// Every field is optional, and an empty outcome is the common one: the
/// message stays in the inbox, unheld and unmarked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// File the message out of the inbox, and why (FR-110 to FR-113).
    pub filter: Option<Reason>,
    /// Hold it for a digest, under this rule (FR-120 to FR-126).
    pub hold: Option<RuleName>,
    /// What it asks of the user: at most one marker per message (FR-100,
    /// FR-104).
    pub marker: Option<MarkerCandidate>,
}

/// Why a message was filtered: the reason shown on its row, and which layer
/// decided it (`filter_decisions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// The category, from the fixed vocabulary.
    pub kind: ReasonKind,
    /// Who it came from, shown after the reason: "notification · Forge".
    pub source: Option<SourceName>,
    /// The layer that decided.
    pub layer: Layer,
}

/// The fixed vocabulary of filter reasons (FR-113). The store's `CHECK`
/// spells the same six.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonKind {
    /// Unsolicited mail.
    Spam,
    /// Marketing.
    Promotion,
    /// An automated notice: an alert, an update, a digest a service sends.
    Notification,
    /// A receipt or an invoice.
    Receipt,
    /// Delivery and tracking.
    Shipping,
    /// A social network's mail.
    Social,
}

impl ReasonKind {
    /// The reason as the store spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReasonKind::Spam => "spam",
            ReasonKind::Promotion => "promotion",
            ReasonKind::Notification => "notification",
            ReasonKind::Receipt => "receipt",
            ReasonKind::Shipping => "shipping",
            ReasonKind::Social => "social",
        }
    }
}

/// Which layer decided a filter reason (`filter_decisions.layer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// The message's own headers: list, bulk and automated signals.
    Header,
    /// The shipped automated-senders table.
    Senders,
    /// The server's own verdict: `$Junk`.
    Server,
    /// The user's model, in milestone 2.
    Model,
}

impl Layer {
    /// The layer as the store spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Layer::Header => "header",
            Layer::Senders => "senders",
            Layer::Server => "server",
            Layer::Model => "model",
        }
    }
}

/// A filter's source: a sender or list name from the senders table or the
/// message's own list header. Only this crate makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceName(pub(crate) String);

impl SourceName {
    /// The name, to store and to show.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `name` of a `[[focus.digests]]` rule, from the user's own config.
/// Only this crate makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleName(pub(crate) String);

impl RuleName {
    /// The name, as the config spells it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Character offsets into a message's own text ([`crate::OwnText`]).
pub type Span = Range<usize>;

/// A marker to draw on the message's row (`markers`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerCandidate {
    /// What the marker says.
    pub kind: MarkerKind,
    /// The sentence it quotes, as offsets into the own text. `None` for an
    /// invitation, which quotes nothing.
    pub span: Option<Span>,
    /// An invitation's event start.
    pub starts_at: Option<DateTime<Utc>>,
    /// An invitation's event end.
    pub ends_at: Option<DateTime<Utc>>,
    /// A to-do's due date, when the sentence names one.
    pub due_at: Option<DateTime<Utc>>,
    /// An invitation's iTIP identity, which an update or a cancellation is
    /// matched by.
    pub invite: Option<InviteIdentity>,
}

/// What a marker says a message asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerKind {
    /// A calendar invitation to answer.
    Invite,
    /// A question put to the user.
    Question,
    /// Something the user is asked to do.
    Todo,
}

/// Which invitation a marker is about (RFC 5546): its `UID`, `SEQUENCE` and
/// `DTSTAMP`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InviteIdentity {
    /// The event's `UID`.
    pub uid: InviteUid,
    /// Its `SEQUENCE`.
    pub sequence: u32,
    /// Its `DTSTAMP`.
    pub stamp: Option<DateTime<Utc>>,
}

/// An invitation's `UID`, as its calendar part gave it. Only this crate makes
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InviteUid(pub(crate) String);

impl InviteUid {
    /// The UID, verbatim.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
