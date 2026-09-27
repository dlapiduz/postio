//! Spike S1 (`specs/007-postio-focus` T007, research R9): calcard, default
//! features off, on the corpus's invitations.
//!
//! Each fixture's `text/calendar` part is taken the way the body backfill
//! will take it -- MIME-decoded by `postio-model`, so base64 and
//! quoted-printable are already undone -- and handed to calcard directly,
//! with nothing of Postio's in between. What is asserted is what an Invite
//! marker needs: the event's start and end as instants, and the zone they
//! were resolved in. The times are chosen so that a wrong zone, or the right
//! zone at the wrong offset, moves the instant, so a bad mapping fails here
//! as a wrong time rather than as a wrong string.

use calcard::common::timezone::Tz;
use calcard::icalendar::dates::TimeOrDelta;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarMethod, ICalendarProperty,
    ICalendarStatus, ICalendarValue,
};
use chrono::{DateTime, Utc};
use postio_model::test_corpus;

/// The fixture's calendar part, decoded, as the adapter will receive it.
fn calendar_text(fixture: &str) -> String {
    let parsed = postio_model::mime::parse(test_corpus::load(fixture).bytes());
    let part = parsed
        .parts
        .iter()
        .find(|part| part.attachment.mime_type == "text/calendar")
        .unwrap_or_else(|| panic!("{fixture}: no text/calendar part"));
    String::from_utf8(part.content.clone())
        .unwrap_or_else(|error| panic!("{fixture}: the calendar part is not UTF-8: {error}"))
}

fn calendar(fixture: &str) -> ICalendar {
    ICalendar::parse(calendar_text(fixture))
        .unwrap_or_else(|entry| panic!("{fixture}: calcard did not parse it: {entry:?}"))
}

fn event(calendar: &ICalendar) -> &ICalendarComponent {
    calendar
        .components
        .iter()
        .find(|component| component.component_type == ICalendarComponentType::VEvent)
        .expect("a VEVENT")
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant in the test")
        .with_timezone(&Utc)
}

/// The first occurrence's start and end, in UTC, and the name of the zone
/// its start was resolved in (`Etc/UTC` for a `Z` time).
fn first_occurrence(fixture: &str) -> (DateTime<Utc>, DateTime<Utc>, String) {
    let calendar = calendar(fixture);
    let expanded = calendar.expand_dates(Tz::Floating, 64);
    assert!(
        expanded.errors.is_empty(),
        "{fixture}: calcard reported {:?}",
        expanded.errors
    );
    let first = expanded
        .events
        .iter()
        .min_by_key(|event| event.start)
        .unwrap_or_else(|| panic!("{fixture}: no occurrence came out"));
    let end = match first.end {
        TimeOrDelta::Time(end) => end,
        TimeOrDelta::Delta(duration) => first.start + duration,
    };
    let zone = first
        .start
        .timezone()
        .name()
        .map(|name| name.into_owned())
        .unwrap_or_else(|| "floating".to_owned());
    (
        first.start.with_timezone(&Utc),
        end.with_timezone(&Utc),
        zone,
    )
}

fn assert_resolves(fixture: &str, start: &str, end: &str, zone: &str) {
    let (got_start, got_end, got_zone) = first_occurrence(fixture);
    assert_eq!(
        (got_start, got_end, got_zone.as_str()),
        (utc(start), utc(end), zone),
        "{fixture}: (start, end, zone)"
    );
}

// --- Start, end and zone, one fixture each ----------------------------------

#[test]
fn a_windows_zone_name_with_its_vtimezone_is_europe_berlin_in_summer_time() {
    // `W. Europe Standard Time`, 10:00 on 6 October: CEST, UTC+2.
    assert_resolves(
        "invite-windows-zone",
        "2026-10-06T08:00:00Z",
        "2026-10-06T08:45:00Z",
        "Europe/Berlin",
    );
}

#[test]
fn a_cancellation_carries_the_same_resolved_times() {
    assert_resolves(
        "invite-cancel",
        "2026-10-06T08:00:00Z",
        "2026-10-06T08:45:00Z",
        "Europe/Berlin",
    );
}

#[test]
fn an_iana_zone_with_its_vtimezone_is_new_york_daylight_time() {
    // 14:00 on 1 October: EDT, UTC-4.
    assert_resolves(
        "invite-iana-zone",
        "2026-10-01T18:00:00Z",
        "2026-10-01T18:30:00Z",
        "America/New_York",
    );
}

#[test]
fn an_update_resolves_its_new_time() {
    // Moved to 10:00 on 2 October, still EDT.
    assert_resolves(
        "invite-update-sequence",
        "2026-10-02T14:00:00Z",
        "2026-10-02T14:30:00Z",
        "America/New_York",
    );
}

#[test]
fn a_quoted_printable_part_after_the_clocks_go_back_is_london_on_gmt() {
    // 09:00 on 3 November: GMT, UTC+0. On BST it would be 08:00.
    assert_resolves(
        "invite-quoted-printable",
        "2026-11-03T09:00:00Z",
        "2026-11-03T09:30:00Z",
        "Europe/London",
    );
}

#[test]
fn utc_times_stay_utc_whatever_a_stray_tzid_property_says() {
    assert_resolves(
        "invite-utc-times",
        "2026-10-15T16:00:00Z",
        "2026-10-15T17:00:00Z",
        "Etc/UTC",
    );
}

#[test]
fn a_weekly_event_starts_on_its_first_occurrence_in_chicago_daylight_time() {
    // 09:30 on 6 October: CDT, UTC-5.
    assert_resolves(
        "invite-weekly-exdate",
        "2026-10-06T14:30:00Z",
        "2026-10-06T14:45:00Z",
        "America/Chicago",
    );
}

#[test]
fn a_tzid_with_no_vtimezone_is_resolved_by_name_at_its_half_hour_offset() {
    // 10:00 in Asia/Kolkata is UTC+5:30. No VTIMEZONE in the calendar says so.
    assert_resolves(
        "invite-zone-without-vtimezone",
        "2026-10-20T04:30:00Z",
        "2026-10-20T05:30:00Z",
        "Asia/Kolkata",
    );
}

#[test]
fn the_corpus_s_older_invite_resolves_stockholm_before_summer_time() {
    // `calendar-invite.eml`, the corpus's first invitation: 14:00 on 12 March
    // is CET, UTC+1.
    assert_resolves(
        "calendar-invite",
        "2026-03-12T13:00:00Z",
        "2026-03-12T14:00:00Z",
        "Europe/Stockholm",
    );
}

// --- Recurrence --------------------------------------------------------------

#[test]
fn a_weekly_rule_skips_its_exdates_and_follows_the_dst_change() {
    let calendar = calendar("invite-weekly-exdate");
    let expanded = calendar.expand_dates(Tz::Floating, 64);
    assert!(expanded.errors.is_empty(), "{:?}", expanded.errors);

    let mut starts: Vec<DateTime<Utc>> = expanded
        .events
        .iter()
        .map(|occurrence| occurrence.start.with_timezone(&Utc))
        .collect();
    starts.sort();

    // Twelve Tuesdays from 6 October to 22 December, less 3 and 24 November.
    // 09:30 in Chicago is 14:30 UTC until 1 November and 15:30 UTC after it.
    let expected: Vec<DateTime<Utc>> = [
        "2026-10-06T14:30:00Z",
        "2026-10-13T14:30:00Z",
        "2026-10-20T14:30:00Z",
        "2026-10-27T14:30:00Z",
        "2026-11-10T15:30:00Z",
        "2026-11-17T15:30:00Z",
        "2026-12-01T15:30:00Z",
        "2026-12-08T15:30:00Z",
        "2026-12-15T15:30:00Z",
        "2026-12-22T15:30:00Z",
    ]
    .into_iter()
    .map(utc)
    .collect();
    assert_eq!(starts, expected);
    assert!(event(&calendar).is_recurrent());
}

// --- The iTIP identity an update and a cancellation are matched by ------------

fn text(component: &ICalendarComponent, property: &ICalendarProperty) -> Option<String> {
    component
        .property(property)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_text)
        .map(str::to_owned)
}

fn integer(component: &ICalendarComponent, property: &ICalendarProperty) -> Option<i64> {
    component
        .property(property)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_integer)
}

fn method(calendar: &ICalendar) -> Option<ICalendarMethod> {
    calendar
        .components
        .iter()
        .find(|component| component.component_type == ICalendarComponentType::VCalendar)
        .and_then(|root| root.property(&ICalendarProperty::Method))
        .and_then(|entry| entry.values.first())
        .and_then(|value| match value {
            ICalendarValue::Method(method) => Some(method.clone()),
            _ => None,
        })
}

#[test]
fn an_update_keeps_the_uid_and_raises_the_sequence() {
    let original = calendar("invite-iana-zone");
    let update = calendar("invite-update-sequence");

    assert_eq!(method(&update), Some(ICalendarMethod::Request));
    assert_eq!(
        text(event(&update), &ICalendarProperty::Uid),
        text(event(&original), &ICalendarProperty::Uid),
    );
    assert_eq!(
        integer(event(&original), &ICalendarProperty::Sequence),
        Some(0)
    );
    assert_eq!(
        integer(event(&update), &ICalendarProperty::Sequence),
        Some(1)
    );
}

#[test]
fn a_cancellation_names_the_request_s_uid_and_says_cancelled() {
    let request = calendar("invite-windows-zone");
    let cancel = calendar("invite-cancel");

    assert_eq!(method(&request), Some(ICalendarMethod::Request));
    assert_eq!(method(&cancel), Some(ICalendarMethod::Cancel));
    assert_eq!(
        text(event(&cancel), &ICalendarProperty::Uid),
        text(event(&request), &ICalendarProperty::Uid),
    );
    assert_eq!(event(&cancel).status(), Some(&ICalendarStatus::Cancelled));
    assert_eq!(
        integer(event(&cancel), &ICalendarProperty::Sequence),
        Some(1)
    );
}

#[test]
fn an_alarm_s_attendee_and_uid_are_not_the_event_s() {
    // Google puts an ATTENDEE inside its VALARM, and Apple a UID: neither is
    // a guest or the event's identity.
    let google = calendar("invite-iana-zone");
    assert_eq!(
        event(&google)
            .properties(&ICalendarProperty::Attendee)
            .count(),
        2,
        "the organizer and Ada, and not the alarm's recipient"
    );

    let apple = calendar("invite-quoted-printable");
    assert_eq!(
        text(event(&apple), &ICalendarProperty::Uid).as_deref(),
        Some("5B2E8F1A-3C4D-4E6F-8A9B-0C1D2E3F4A5B")
    );
}
