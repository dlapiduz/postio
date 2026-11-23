//! Classification for Postio Focus: one fixed-schema answer per message, decided in layers with the
//! guards first. This crate cannot send mail: nothing that sends is in its dependency closure (ADR
//! 0009). [`at_filing`] and [`at_body`] are the two moments a message is classified; [`ModelLayer`]
//! is where the user's own model answers, with the built-in detector as fallback.

/// Raise it when a rule change should classify stored mail again: every record at an older version
/// is then due.
pub const VERSION: u32 = 1;

pub mod bounce;
mod digests;
pub mod duplicates;
mod facts;
mod filters;
mod guards;
mod input;
pub mod kind;
mod needs_action;
mod outcome;
mod pipeline;
mod rules;
mod senders;
pub mod signature;
pub mod tally;

pub use digests::Digests;
pub use facts::Facts;
pub use input::{BodyMessage, EXCERPT_CHARS, FiledMessage, OwnText};
pub use needs_action::considered;
pub use outcome::{
    InviteIdentity, InviteUid, Layer, MarkedBy, MarkerCandidate, MarkerKind, Outcome, Reason,
    ReasonKind, RuleName, SourceName, Span,
};
pub use pipeline::{ModelLayer, NeedsAction, Unavailable, at_body, at_body_with, at_filing};
pub use rules::Rules;
pub use senders::{Sender, Senders, SendersError};
pub use tally::Tally;
