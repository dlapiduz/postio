//! Calendar objects and the invitations mail carries: read, expanded and answered. A pure leaf;
//! no library type crosses its edge.

mod edit;
mod event;
mod expand;
mod ics;
mod invitation;
mod parse;
mod reply;
#[cfg(test)]
mod test_support;

pub use edit::{Edited, Edits, NewEvent, Scope, new_object, patch, remove, request, with_uid};
pub use event::{Alarm, AlarmAction, Event, Status, Trigger, parse_object};
pub use expand::{Occurrence, expand};
pub use invitation::{Answer, Attendee, EventTime, Invitation, Method, PartStat, Zone, supersedes};
pub use parse::{CalendarError, parse};
pub use reply::reply;

/// Past it one object's expansion stops: a hostile rule cannot fill the store.
pub const MAX_OCCURRENCES: usize = 4_096;
