//! What an invitation says, in Postio's own types.
//!
//! Nothing here is calcard's: the adapter translates at the edge, so a
//! change of parser (research R9 expects calcard 0.4 to move to jiff) stays
//! inside `parse.rs` and `reply.rs`.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use postio_model::EmailAddress;

/// One event from a `text/calendar` part: what an Invite marker shows and
/// what an RSVP needs (`specs/007-postio-focus` data-model, "Invitation").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    /// `UID`: the event's identity. An update and a cancellation carry the
    /// same one (RFC 5546 §2.1.5).
    pub uid: String,
    /// `SEQUENCE`, `0` when absent. The organiser raises it with every
    /// change that matters to attendees.
    pub sequence: u32,
    /// `DTSTAMP`: when this version was written. It orders two versions with
    /// the same `sequence`.
    pub stamp: Option<DateTime<Utc>>,
    /// What the calendar asks of its recipients.
    pub method: Method,
    /// The event is called off: a `CANCEL`, or `STATUS:CANCELLED` on the
    /// event itself, which some servers send inside a `REQUEST`.
    pub cancelled: bool,
    /// `SUMMARY`, unescaped.
    pub summary: Option<String>,
    /// `DTSTART`, or the first occurrence's start for a recurring event.
    pub starts_at: EventTime,
    /// `DTEND`, or the start plus `DURATION`, or the RFC 5545 default when
    /// the calendar gives neither: a day for a date, no time for a time.
    pub ends_at: EventTime,
    /// The zone the times were given in.
    pub zone: Zone,
    /// `LOCATION`, unescaped.
    pub location: Option<String>,
    /// `ORGANIZER`: who a reply goes to.
    pub organizer: Option<EmailAddress>,
    /// The event's `ATTENDEE`s, and only the event's: an `ATTENDEE` inside a
    /// `VALARM` is who the alarm notifies, not a guest.
    pub attendees: Vec<Attendee>,
    /// The event repeats (`RRULE` or `RDATE`).
    pub recurring: bool,
    /// `RECURRENCE-ID`: the one occurrence of a series this invitation is
    /// about, when it is about one. A reply to it has to name it too.
    pub recurrence_id: Option<EventTime>,
    /// When a recurring event's last occurrence ends, for a series whose
    /// rule says it ends (`COUNT` or `UNTIL`, or a set of `RDATE`s).
    /// `None` for a series with no end, for one longer than the adapter
    /// walks ([`crate::parse`]), and for an event that does not recur.
    pub series_ends_at: Option<EventTime>,
}

impl Invitation {
    /// When the event is over for good: when it ends, or when a series'
    /// last occurrence does. `None` for a series that never ends. An
    /// invitation to an event that is over offers no answer (FR-103).
    pub fn last_end(&self) -> Option<EventTime> {
        if self.recurring {
            self.series_ends_at
        } else {
            Some(self.ends_at)
        }
    }
}

/// What a calendar asks of its recipients (RFC 5546 `METHOD`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// An invitation, or an update to one. It can be answered.
    Request,
    /// The organiser called the event, or one occurrence of it, off.
    Cancel,
    /// Anything else -- a `PUBLISH`, someone's `REPLY`, a `COUNTER`, or no
    /// `METHOD` at all. There is nothing to answer.
    Other,
}

/// An attendee's answer, as `PARTSTAT` spells it (RFC 5545 §3.2.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartStat {
    /// Not answered yet, which is also what an absent `PARTSTAT` means.
    NeedsAction,
    /// Accepted.
    Accepted,
    /// Declined.
    Declined,
    /// Tentatively accepted.
    Tentative,
    /// Handed on to somebody else.
    Delegated,
    /// A status that belongs to tasks rather than events.
    Other,
}

/// One `ATTENDEE` of the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    /// The attendee's address, with the `CN` as its display name.
    pub address: EmailAddress,
    /// Their answer so far.
    pub partstat: PartStat,
}

/// The answer the user gives from the row or the open message (`y`, `Y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    /// Accept: `PARTSTAT=ACCEPTED`.
    Accept,
    /// Decline: `PARTSTAT=DECLINED`.
    Decline,
}

/// A start or an end, as the calendar gave it.
///
/// Only an [`EventTime::At`] is an instant on its own. A floating time and a
/// whole day read the same wherever the user is, so they become instants only
/// in the user's zone, through [`EventTime::instant_in`]. Keeping the three
/// apart is what stops a floating 10:00 being shown as 10:00 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTime {
    /// A fixed instant: a UTC time, or a local time in a zone that resolved.
    At(DateTime<Utc>),
    /// A local time with no zone (RFC 5545 §3.3.5, form 1), or in a zone
    /// nothing could resolve ([`Zone::Unknown`]).
    Floating(NaiveDateTime),
    /// A whole day (`VALUE=DATE`).
    Day(NaiveDate),
}

impl EventTime {
    /// This time as an instant, reading a floating time or a whole day in
    /// `zone`, the user's own.
    ///
    /// A local time that `zone` skips, in the hour its clocks go forward, is
    /// read with the offset before the gap, as RFC 5545 §3.3.5 says; one it
    /// repeats is read as the first of the two.
    pub fn instant_in<Z: TimeZone>(&self, zone: &Z) -> DateTime<Utc> {
        match self {
            EventTime::At(instant) => *instant,
            EventTime::Floating(local) => local_instant(zone, *local),
            EventTime::Day(day) => local_instant(zone, day.and_time(NaiveTime::MIN)),
        }
    }
}

/// `local` on `zone`'s clock, as an instant (RFC 5545 §3.3.5).
///
/// A reading the clocks repeat is the first of the two. One they skip is read
/// with the offset in force before the gap: no zone's clocks jump by more
/// than a few hours, so three hours earlier is always before it.
pub(crate) fn local_instant<Z: TimeZone>(zone: &Z, local: NaiveDateTime) -> DateTime<Utc> {
    if let Some(instant) = zone.from_local_datetime(&local).earliest() {
        return instant.with_timezone(&Utc);
    }
    let before = Duration::hours(3);
    match zone.from_local_datetime(&(local - before)).earliest() {
        Some(earlier) => (earlier + before).with_timezone(&Utc),
        None => local.and_utc(),
    }
}

/// The zone an event's times were given in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Zone {
    /// UTC times (a `Z` suffix, or a `TZID` naming UTC).
    Utc,
    /// A zone that resolved, by its IANA name: from the calendar's own
    /// `VTIMEZONE`, from an IANA `TZID`, or from a Windows zone name mapped
    /// to its IANA equivalent. A `TZID` whose `VTIMEZONE` is missing still
    /// resolves this way when the name is one of those. A time written with a
    /// numeric offset, which RFC 5545 does not allow and some generators
    /// write anyway, is named by the offset alone: `UTC+05:30`.
    Named(String),
    /// No zone at all: floating times, or whole days.
    Floating,
    /// A `TZID` nothing could resolve, verbatim. The times are its wall-clock
    /// readings, carried as [`EventTime::Floating`], because guessing an
    /// offset would show a time that looks right and is not.
    Unknown(String),
}

/// Whether `newer` replaces `older` as the latest word on one event.
///
/// Both must be about the same event (the same `UID`) and the same
/// occurrence of it (the same `RECURRENCE-ID`, or neither). Then the higher
/// `SEQUENCE` wins, and on a tie the later `DTSTAMP` does (RFC 5546 §2.1.5,
/// research R9). With equal sequences and a stamp missing from either side
/// there is no telling which came later, so nothing is replaced.
pub fn supersedes(newer: &Invitation, older: &Invitation) -> bool {
    if newer.uid != older.uid || newer.recurrence_id != older.recurrence_id {
        return false;
    }
    match newer.sequence.cmp(&older.sequence) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            matches!((newer.stamp, older.stamp), (Some(newer), Some(older)) if newer > older)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use calcard::common::timezone::Tz;
    use chrono::FixedOffset;

    use super::*;
    use crate::test_support::{at, day, floating, invitation, utc};

    // --- supersedes -------------------------------------------------------------

    #[test]
    fn an_update_at_a_higher_sequence_supersedes_the_request() {
        let original = invitation("invite-iana-zone");
        let update = invitation("invite-update-sequence");

        assert!(supersedes(&update, &original));
        assert!(!supersedes(&original, &update), "and never the other way");
    }

    #[test]
    fn a_cancellation_supersedes_the_request_it_cancels() {
        let request = invitation("invite-windows-zone");
        let cancel = invitation("invite-cancel");

        assert!(supersedes(&cancel, &request));
    }

    #[test]
    fn on_equal_sequences_the_later_stamp_wins() {
        let older = invitation("invite-iana-zone");
        let mut newer = older.clone();
        newer.stamp = Some(utc("2026-09-27T16:00:00Z"));

        assert!(supersedes(&newer, &older));
        assert!(!supersedes(&older, &newer));
    }

    #[test]
    fn an_invitation_does_not_supersede_itself() {
        let invite = invitation("invite-iana-zone");

        assert!(!supersedes(&invite, &invite));
    }

    #[test]
    fn on_equal_sequences_a_missing_stamp_decides_nothing() {
        let with = invitation("invite-iana-zone");
        let mut without = with.clone();
        without.stamp = None;

        assert!(!supersedes(&without, &with));
        assert!(!supersedes(&with, &without));
    }

    #[test]
    fn another_event_never_supersedes_this_one() {
        let request = invitation("invite-windows-zone");
        let mut other = invitation("invite-cancel");
        other.uid = "a-different-event@example.com".to_owned();

        assert!(!supersedes(&other, &request));
    }

    #[test]
    fn an_update_to_one_occurrence_does_not_supersede_the_series() {
        let series = invitation("invite-weekly-exdate");
        let mut occurrence = series.clone();
        occurrence.sequence = 1;
        occurrence.recurrence_id = Some(at("2026-10-13T14:30:00Z"));

        assert!(!supersedes(&occurrence, &series));
    }

    // --- instant_in ---------------------------------------------------------------

    fn zone(name: &str) -> Tz {
        Tz::from_str(name).expect("a zone the test names")
    }

    #[test]
    fn an_instant_is_the_same_in_every_zone() {
        let meeting = at("2026-10-06T08:00:00Z");

        assert_eq!(
            meeting.instant_in(&zone("Asia/Tokyo")),
            utc("2026-10-06T08:00:00Z")
        );
    }

    #[test]
    fn a_floating_time_is_read_on_the_user_s_clock() {
        let ten = floating("2026-10-06T10:00:00");

        assert_eq!(
            ten.instant_in(&zone("America/New_York")),
            utc("2026-10-06T14:00:00Z")
        );
        assert_eq!(
            ten.instant_in(&FixedOffset::east_opt(2 * 3600).expect("an offset")),
            utc("2026-10-06T08:00:00Z")
        );
    }

    #[test]
    fn a_whole_day_starts_at_the_user_s_midnight() {
        assert_eq!(
            day("2026-10-15").instant_in(&zone("Europe/Berlin")),
            utc("2026-10-14T22:00:00Z")
        );
    }

    #[test]
    fn a_time_the_clocks_skip_is_read_with_the_offset_before_the_gap() {
        // New York skips 02:00-03:00 on 8 March 2026. RFC 5545 §3.3.5 reads
        // 02:30 with the offset before the gap, EST: 07:30 UTC.
        assert_eq!(
            floating("2026-03-08T02:30:00").instant_in(&zone("America/New_York")),
            utc("2026-03-08T07:30:00Z")
        );
    }

    #[test]
    fn a_time_the_clocks_repeat_is_the_first_of_the_two() {
        // 01:30 happens twice in New York on 1 November 2026; the first is EDT.
        assert_eq!(
            floating("2026-11-01T01:30:00").instant_in(&zone("America/New_York")),
            utc("2026-11-01T05:30:00Z")
        );
    }
}
