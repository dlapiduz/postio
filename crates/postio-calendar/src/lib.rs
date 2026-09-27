//! Calendar invitations carried in mail (`specs/007-postio-focus`,
//! research R9).
//!
//! It parses a `text/calendar` part into an invitation and writes the reply
//! that answers one (RFC 5545 and RFC 5546). It is a pure leaf: no store, no
//! toolkit, no network.
//!
//! Three calls are the whole of it, as `contracts/engine.md` names them:
//!
//! - [`parse`]: a calendar part's bytes to an [`Invitation`], with its start
//!   and end resolved to instants wherever the calendar says where they are;
//! - [`reply()`]: the `METHOD:REPLY` that accepts or declines one;
//! - [`supersedes`]: whether one version of an event replaces another.
//!
//! calcard does the reading and the writing (spike S1 chose it over
//! Pimalaya's ical-rs for its zone resolution). None of its types cross this
//! crate's edge, so the parser can change without its callers noticing.

mod invitation;
mod parse;
mod reply;
#[cfg(test)]
mod test_support;

pub use invitation::{Answer, Attendee, EventTime, Invitation, Method, PartStat, Zone, supersedes};
pub use parse::{CalendarError, parse};
pub use reply::reply;
