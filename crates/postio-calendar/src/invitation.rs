//! What an invitation says, in Postio's own types; none of calcard's cross the crate edge.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use postio_model::EmailAddress;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub uid: String,
    pub sequence: u32,
    pub stamp: Option<DateTime<Utc>>,
    pub method: Method,
    /// A `CANCEL`, or `STATUS:CANCELLED` on the event itself, which some servers send inside a
    /// `REQUEST`.
    pub cancelled: bool,
    pub summary: Option<String>,
    pub starts_at: EventTime,
    /// `DTEND`, or start plus `DURATION`, or the RFC 5545 default: a day for a date, no time for a
    /// time.
    pub ends_at: EventTime,
    pub zone: Zone,
    pub location: Option<String>,
    pub organizer: Option<EmailAddress>,
    /// Only the event's: an `ATTENDEE` inside a `VALARM` is who the alarm notifies, not a guest.
    pub attendees: Vec<Attendee>,
    pub recurring: bool,
    /// `RECURRENCE-ID`: the one occurrence this invitation is about; a reply has to name it too.
    pub recurrence_id: Option<EventTime>,
    /// `None` for a series with no end, one longer than the adapter walks, and an event that does
    /// not recur.
    pub series_ends_at: Option<EventTime>,
}

impl Invitation {
    /// `None` for a series that never ends. An invitation to an event that is over offers no
    /// answer.
    pub fn last_end(&self) -> Option<EventTime> {
        if self.recurring {
            self.series_ends_at
        } else {
            Some(self.ends_at)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Request,
    Cancel,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartStat {
    /// Not answered yet; also what an absent `PARTSTAT` means.
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
    Delegated,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    pub address: EmailAddress,
    pub partstat: PartStat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Accept,
    Decline,
}

/// Only `At` is an instant on its own; a floating 10:00 must not be shown as 10:00 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTime {
    At(DateTime<Utc>),
    /// A local time with no zone, or in a zone nothing could resolve (`Zone::Unknown`).
    Floating(NaiveDateTime),
    Day(NaiveDate),
}

impl EventTime {
    /// A local time `zone` skips is read with the offset before the gap (RFC 5545 §3.3.5); one it
    /// repeats, as the first of the two.
    pub fn instant_in<Z: TimeZone>(&self, zone: &Z) -> DateTime<Utc> {
        match self {
            EventTime::At(instant) => *instant,
            EventTime::Floating(local) => local_instant(zone, *local),
            EventTime::Day(day) => local_instant(zone, day.and_time(NaiveTime::MIN)),
        }
    }
}

/// A skipped time is read three hours earlier, which is always before the gap: no zone's clocks
/// jump by more.
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Zone {
    Utc,
    /// By IANA name, from the `VTIMEZONE`, an IANA `TZID` or a mapped Windows name. A numeric
    /// offset, which RFC 5545 disallows but some generators write, is named by the offset alone:
    /// `UTC+05:30`.
    Named(String),
    Floating,
    /// A `TZID` nothing could resolve, verbatim; carried as `Floating` because a guessed offset
    /// would show a time that looks right and is not.
    Unknown(String),
}

/// Same `UID` and `RECURRENCE-ID`; then higher `SEQUENCE` wins, then later `DTSTAMP`. With equal
/// sequences and a missing stamp nothing is replaced.
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
        assert_eq!(
            floating("2026-03-08T02:30:00").instant_in(&zone("America/New_York")),
            utc("2026-03-08T07:30:00Z")
        );
    }

    #[test]
    fn a_time_the_clocks_repeat_is_the_first_of_the_two() {
        assert_eq!(
            floating("2026-11-01T01:30:00").instant_in(&zone("America/New_York")),
            utc("2026-11-01T05:30:00Z")
        );
    }
}
