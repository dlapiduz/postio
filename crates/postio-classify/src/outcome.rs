//! A fixed schema with no text of its own (ADR 0009): a model's words are attacker-shaped by the
//! time
//! they reach a row. A marker's quote is a `Span` of the message's own text; the names an outcome
//! carries are newtypes this crate alone constructs.

use std::ops::Range;

use chrono::{DateTime, Utc};

/// Every field is optional; an empty outcome is the common one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub filter: Option<Reason>,
    pub hold: Option<RuleName>,
    pub marker: Option<MarkerCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub kind: ReasonKind,
    pub source: Option<SourceName>,
    pub layer: Layer,
}

/// The store's `CHECK` spells the same six.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonKind {
    Spam,
    Promotion,
    Notification,
    Receipt,
    Shipping,
    Social,
}

impl ReasonKind {
    /// Every reason, in the order the Filtered view's tabs list them.
    pub const ALL: [ReasonKind; 6] = [
        ReasonKind::Spam,
        ReasonKind::Promotion,
        ReasonKind::Notification,
        ReasonKind::Receipt,
        ReasonKind::Shipping,
        ReasonKind::Social,
    ];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Header,
    /// The shipped automated-senders table.
    Senders,
    Server,
    Model,
}

impl Layer {
    pub const ALL: [Layer; 4] = [Layer::Header, Layer::Senders, Layer::Server, Layer::Model];

    pub const fn as_str(self) -> &'static str {
        match self {
            Layer::Header => "header",
            Layer::Senders => "senders",
            Layer::Server => "server",
            Layer::Model => "model",
        }
    }
}

/// A sender or list name from the senders table or the message's own list header. Only this crate
/// makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceName(pub(crate) String);

impl SourceName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// From the user's own config. Only this crate makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleName(pub(crate) String);

impl RuleName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub type Span = Range<usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerCandidate {
    pub kind: MarkerKind,
    /// `None` for an invitation, which quotes nothing.
    pub span: Option<Span>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub due_at: Option<DateTime<Utc>>,
    pub invite: Option<InviteIdentity>,
    /// What the store records as the marker's source: the person's model and the built-in detector
    /// answer the same question in turn.
    pub by: MarkedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkedBy {
    Calendar,
    Detector,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerKind {
    Invite,
    Question,
    Todo,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InviteIdentity {
    pub uid: InviteUid,
    pub sequence: u32,
    pub stamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InviteUid(pub(crate) String);

impl InviteUid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
