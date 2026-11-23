use std::ops::Range;

use chrono::{DateTime, Utc};

use crate::CalendarError;

/// An occurrence's times are UTC, except a floating or all-day one's, which are its wall clock
/// read as UTC: resolved on the reader's clock when shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub uid: String,
    /// The instance's identity as `RECURRENCE-ID` names it; `None` for an event that does not recur.
    pub recurrence_id: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub floating: bool,
}

/// Every occurrence of every event in `ics` that overlaps `window`, in start order. The same
/// window gives the same occurrences, so materialising twice, or in pieces, adds nothing new.
pub fn expand(ics: &[u8], window: Range<DateTime<Utc>>) -> Result<Vec<Occurrence>, CalendarError> {
    let _ = (ics, window);
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::test_support::utc;

    const DST: &[u8] = include_bytes!("../tests/fixtures/weekly-dst-exception.ics");
    const ALL_DAY: &[u8] = include_bytes!("../tests/fixtures/all-day-multi-day.ics");
    const FLOATING: &[u8] = include_bytes!("../tests/fixtures/floating.ics");
    const WINDOWS: &[u8] = include_bytes!("../tests/fixtures/windows-zone.ics");
    const NO_VTIMEZONE: &[u8] = include_bytes!("../tests/fixtures/zone-without-vtimezone.ics");
    const VTODO: &[u8] = include_bytes!("../tests/fixtures/unknown-properties.ics");

    fn autumn() -> Range<DateTime<Utc>> {
        utc("2026-09-01T00:00:00Z")..utc("2027-01-01T00:00:00Z")
    }

    fn starts(ics: &[u8], window: Range<DateTime<Utc>>) -> Vec<String> {
        expand(ics, window)
            .expect("expands")
            .iter()
            .map(|occurrence| occurrence.start.format("%m-%d %H:%M").to_string())
            .collect()
    }

    #[test]
    fn a_weekly_rule_keeps_its_local_hour_across_the_end_of_daylight_time() {
        assert_eq!(
            starts(DST, autumn()),
            [
                "10-06 13:30",
                "10-13 13:30",
                "10-27 18:00",
                "11-03 14:30",
                "11-17 14:30",
                "11-24 14:30"
            ]
        );
    }

    #[test]
    fn an_excluded_date_is_absent_and_a_moved_one_keeps_its_identity() {
        let occurrences = expand(DST, autumn()).expect("expands");
        let moved = occurrences
            .iter()
            .find(|occurrence| occurrence.start == utc("2026-10-27T18:00:00Z"))
            .expect("the moved occurrence");

        assert_eq!(moved.recurrence_id.as_deref(), Some("20261027T093000"));
        assert_eq!(moved.end - moved.start, TimeDelta::minutes(30));
        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.start.format("%m-%d").to_string() != "10-20")
        );
    }

    #[test]
    fn an_all_day_event_spans_its_dates_on_no_clock() {
        let occurrences = expand(ALL_DAY, autumn()).expect("expands");

        assert_eq!(occurrences.len(), 1);
        assert!(occurrences[0].all_day);
        assert_eq!(occurrences[0].start, utc("2026-12-14T00:00:00Z"));
        assert_eq!(occurrences[0].end, utc("2026-12-17T00:00:00Z"));
        assert_eq!(occurrences[0].recurrence_id, None);
    }

    #[test]
    fn a_floating_time_stays_at_its_wall_clock() {
        let occurrences = expand(FLOATING, autumn()).expect("expands");

        assert_eq!(
            starts(FLOATING, autumn()),
            ["12-03 07:00", "12-04 07:00", "12-05 07:00"]
        );
        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.floating && !occurrence.all_day)
        );
    }

    #[test]
    fn a_windows_zone_name_resolves_without_a_vtimezone() {
        assert_eq!(starts(WINDOWS, autumn()), ["10-30 16:00", "11-06 17:00"]);
    }

    #[test]
    fn an_iana_zone_the_object_does_not_define_still_resolves() {
        assert_eq!(
            starts(NO_VTIMEZONE, autumn()),
            ["10-24 09:00", "10-31 10:00"]
        );
    }

    #[test]
    fn a_task_is_not_an_occurrence() {
        assert_eq!(starts(VTODO, autumn()), ["11-07 10:00"]);
    }

    #[test]
    fn expanding_in_two_pieces_gives_what_one_window_gives() {
        let whole = expand(DST, autumn()).expect("expands");
        let mut pieces = expand(
            DST,
            utc("2026-09-01T00:00:00Z")..utc("2026-11-01T00:00:00Z"),
        )
        .expect("expands");
        pieces.extend(
            expand(
                DST,
                utc("2026-11-01T00:00:00Z")..utc("2027-01-01T00:00:00Z"),
            )
            .expect("expands"),
        );
        pieces.dedup();

        assert_eq!(pieces, whole);
        assert_eq!(expand(DST, autumn()).expect("expands"), whole);
    }

    #[test]
    fn a_rule_with_no_end_yields_only_the_window() {
        let daily = crate::test_support::ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Postio tests//EN",
            "BEGIN:VEVENT",
            "UID:forever@example.com",
            "DTSTAMP:19700101T000000Z",
            "DTSTART:19700101T090000Z",
            "DURATION:PT15M",
            "RRULE:FREQ=DAILY",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);

        let week = expand(
            &daily,
            utc("2026-11-23T00:00:00Z")..utc("2026-11-30T00:00:00Z"),
        )
        .expect("expands");

        assert_eq!(week.len(), 7);
        assert_eq!(week[0].recurrence_id.as_deref(), Some("20261123T090000Z"));
    }

    #[test]
    fn a_rule_that_would_flood_the_window_is_cut_short() {
        let flood = crate::test_support::ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Postio tests//EN",
            "BEGIN:VEVENT",
            "UID:flood@example.com",
            "DTSTAMP:20260101T000000Z",
            "DTSTART:20260101T000000Z",
            "RRULE:FREQ=SECONDLY",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);

        let occurrences = expand(&flood, autumn()).expect("expands");

        assert_eq!(occurrences.len(), crate::MAX_OCCURRENCES);
    }

    #[test]
    fn a_part_that_is_not_a_calendar_is_refused() {
        assert_eq!(
            expand(b"not a calendar", autumn()),
            Err(CalendarError::NotACalendar)
        );
    }
}
