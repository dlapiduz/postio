//! Classification for Postio Focus (`specs/007-postio-focus`, research R8).
//!
//! One fixed-schema answer per message: whether to file it away as spam or an
//! update, whether to hold it for a digest, and what it asks of the user. It
//! is decided in layers, and the guards come first. This crate cannot send
//! mail. Nothing that sends is in its dependency closure, and
//! `scripts/checks/check-crate-boundaries.py` holds that line (ADR 0009).
//!
//! - [`at_filing`] and [`at_body`] are the two moments a message is
//!   classified: as it is filed, and when its body arrives;
//! - [`Facts`] answers the guards, and [`Rules`] is everything decided by,
//!   as data: [`Senders`] is the automated-senders table Postio ships;
//! - [`Outcome`] is the answer, a schema with no text of its own (FR-132);
//! - the built-in needs-action detector finds a question or a to-do in the
//!   own text of mail sent directly to the user (FR-104 to FR-106, research
//!   R10). It is held to SC-013's precision by `tests/needs_action.rs`;
//! - [`ModelLayer`] is where the user's own model answers, in milestone 2,
//!   for what the built-in layers leave open, and in the detector's place
//!   ([`at_body_with`]); when it is not running, the detector answers.

/// The classifier's version: what `focus_classified` records a message was
/// classified by. Raise it when a change to the rules should classify the
/// mail already stored again: every record at an older version is then due,
/// and the catch-up at Focus's start takes it (FR-141).
pub const VERSION: u32 = 1;

mod digests;
mod facts;
mod filters;
mod guards;
mod input;
mod needs_action;
mod outcome;
mod pipeline;
mod rules;
mod senders;

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
