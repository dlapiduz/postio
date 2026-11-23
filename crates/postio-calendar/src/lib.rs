//! Calendar invitations carried in mail: parses a `text/calendar` part and writes the
//! `METHOD:REPLY` that answers one. A pure leaf; none of calcard's types cross its edge.

mod expand;
mod invitation;
mod parse;
mod reply;
#[cfg(test)]
mod test_support;

pub use expand::{Occurrence, expand};
pub use invitation::{Answer, Attendee, EventTime, Invitation, Method, PartStat, Zone, supersedes};
pub use parse::{CalendarError, parse};
pub use reply::reply;

/// Past it one object's expansion stops: a hostile rule cannot fill the store.
pub const MAX_OCCURRENCES: usize = 4_096;
