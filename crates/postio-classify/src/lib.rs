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
//!   as data;
//! - [`Outcome`] is the answer, a schema with no text of its own (FR-132);
//! - the built-in needs-action detector finds a question or a to-do in the
//!   own text of mail sent directly to the user (FR-104 to FR-106, research
//!   R10). It is held to SC-013's precision by `tests/needs_action.rs`;
//! - [`ModelLayer`] is where the user's own model will answer, in milestone
//!   2, for what the built-in layers leave open, and in the detector's place.

mod facts;
mod input;
mod needs_action;
mod outcome;
mod pipeline;
mod rules;

pub use facts::Facts;
pub use input::{BodyMessage, FiledMessage, OwnText};
pub use outcome::{
    InviteIdentity, InviteUid, Layer, MarkerCandidate, MarkerKind, Outcome, Reason, ReasonKind,
    RuleName, SourceName, Span,
};
pub use pipeline::{ModelLayer, NeedsAction, at_body, at_filing};
pub use rules::Rules;
