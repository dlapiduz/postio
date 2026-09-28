//! From a `text/calendar` part to an [`Invitation`], through calcard.

use std::borrow::Cow;

use calcard::common::timezone::Tz;
use calcard::icalendar::dates::TimeOrDelta;
use calcard::icalendar::timezone::TzResolver;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarMethod,
    ICalendarParameterName, ICalendarParameterValue, ICalendarParticipationStatus,
    ICalendarProperty, ICalendarStatus, ICalendarValue,
};
use chrono::{DateTime, Duration, Utc};
use postio_model::EmailAddress;

use crate::invitation::{Attendee, EventTime, Invitation, Method, PartStat, Zone, local_instant};

/// Why a calendar part yielded no invitation.
///
/// Every variant names a structural absence. Nothing in the message's text
/// is carried, so an error is safe to log (FR-151).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    /// The part is not an iCalendar object at all.
    #[error("the part is not an iCalendar object")]
    NotACalendar,
    /// The calendar carries no `VEVENT`: a task, a free/busy reply, or only
    /// time zones.
    #[error("the calendar carries no event")]
    NoEvent,
    /// The event has no `UID`, so no update or cancellation could ever find
    /// it again.
    #[error("the event has no UID")]
    NoUid,
    /// The event has no usable `DTSTART`.
    #[error("the event has no start")]
    NoStart,
}

/// Parses a `text/calendar` part's decoded bytes into the invitation it
/// carries.
///
/// The bytes are the part after its transfer encoding is undone, as the body
/// backfill stores them. They are read as UTF-8, which RFC 5545 requires, and
/// a stray invalid byte is replaced rather than costing the whole invitation.
///
/// When the calendar holds a series and overrides of it, the invitation is
/// the series: the `VEVENT` without a `RECURRENCE-ID`. When it holds only
/// overrides -- an update to one occurrence -- it is the first of them.
///
/// # Errors
///
/// [`CalendarError`] when there is no calendar, no event, or an event with
/// no `UID` or no start.
pub fn parse(ics: &[u8]) -> Result<Invitation, CalendarError> {
    let text = String::from_utf8_lossy(ics);
    let calendar = ICalendar::parse(text.as_ref()).map_err(|_| CalendarError::NotACalendar)?;

    let events: Vec<&ICalendarComponent> = calendar
        .components
        .iter()
        .filter(|component| component.component_type == ICalendarComponentType::VEvent)
        .collect();
    let event = events
        .iter()
        .find(|event| !event.is_recurrence_override())
        .or_else(|| events.first())
        .ok_or(CalendarError::NoEvent)?;

    let uid = event
        .uid()
        .map(str::trim)
        .filter(|uid| !uid.is_empty())
        .ok_or(CalendarError::NoUid)?
        .to_owned();

    // Every VTIMEZONE the calendar carries, by TZID, falling back on the
    // TZID as a name -- IANA, or Windows mapped to IANA -- when there is none.
    let zones = calendar.build_tz_resolver();
    let (starts_at, zone) = event
        .property(&ICalendarProperty::Dtstart)
        .and_then(|entry| time(entry, &zones))
        .ok_or(CalendarError::NoStart)?;
    let ends_at = event
        .property(&ICalendarProperty::Dtend)
        .and_then(|entry| time(entry, &zones))
        .map(|(end, _)| end)
        .or_else(|| duration(event).map(|length| later(starts_at, length)))
        .unwrap_or_else(|| default_end(starts_at));

    let method = method(&calendar);
    Ok(Invitation {
        uid,
        sequence: sequence(event),
        stamp: event.property(&ICalendarProperty::Dtstamp).and_then(stamp),
        method,
        cancelled: method == Method::Cancel || event.status() == Some(&ICalendarStatus::Cancelled),
        summary: text_of(event, &ICalendarProperty::Summary),
        starts_at,
        ends_at,
        zone,
        location: text_of(event, &ICalendarProperty::Location),
        organizer: event
            .property(&ICalendarProperty::Organizer)
            .and_then(address),
        attendees: event
            .properties(&ICalendarProperty::Attendee)
            .filter_map(|entry| {
                Some(Attendee {
                    address: address(entry)?,
                    partstat: partstat(entry),
                })
            })
            .collect(),
        recurring: event.is_recurrent(),
        recurrence_id: event
            .property(&ICalendarProperty::RecurrenceId)
            .and_then(|entry| time(entry, &zones))
            .map(|(occurrence, _)| occurrence),
        series_ends_at: series_end(&calendar, event),
    })
}

/// The most occurrences of a series the adapter walks to find its last. A
/// weekly meeting for twenty years is under it; past it, a series is read
/// as one with no end, whose invitation is never over.
const OCCURRENCES_READ: usize = 1_024;

/// When `event`'s last occurrence ends, for a recurring event whose rules
/// all end: every `RRULE` with a `COUNT` or an `UNTIL`, and `RDATE`s, which
/// are a list. Floating times stay floating, to be read on the user's clock.
fn series_end(calendar: &ICalendar, event: &ICalendarComponent) -> Option<EventTime> {
    if !event.is_recurrent() {
        return None;
    }
    let ends = event.properties(&ICalendarProperty::Rrule).all(|entry| {
        entry.values.iter().all(|value| match value {
            ICalendarValue::RecurrenceRule(rule) => rule.until.is_some() || rule.count.is_some(),
            _ => true,
        })
    });
    if !ends {
        return None;
    }
    let position = calendar
        .components
        .iter()
        .position(|component| std::ptr::eq(component, event))?;
    // calcard walks a series in its default zone unless the start names a
    // TZID, so the default is the start's own: UTC for a `Z` time, its
    // offset for one written with one, and floating otherwise.
    let own_zone = event
        .property(&ICalendarProperty::Dtstart)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_partial_date_time)
        .and_then(|value| value.to_date_time())
        .and_then(|reading| reading.tz())
        .unwrap_or(Tz::Floating);
    let expanded = calendar.expand_dates(own_zone, OCCURRENCES_READ + 1);
    let occurrences: Vec<_> = expanded
        .events
        .iter()
        .filter(|occurrence| occurrence.comp_id as usize == position)
        .collect();
    if occurrences.is_empty() || occurrences.len() > OCCURRENCES_READ {
        return None;
    }
    occurrences
        .iter()
        .map(|occurrence| match occurrence.end {
            TimeOrDelta::Time(end) => end,
            TimeOrDelta::Delta(length) => occurrence.start + length,
        })
        .max_by_key(|end| end.naive_utc())
        .map(|end| {
            if end.timezone().is_floating() {
                EventTime::Floating(end.naive_local())
            } else {
                EventTime::At(end.with_timezone(&Utc))
            }
        })
}

/// The calendar's `METHOD`, which lives on the `VCALENDAR` itself.
fn method(calendar: &ICalendar) -> Method {
    let method = calendar
        .components
        .iter()
        .filter(|component| component.component_type == ICalendarComponentType::VCalendar)
        .find_map(|root| root.property(&ICalendarProperty::Method))
        .and_then(|entry| entry.values.first());
    match method {
        Some(ICalendarValue::Method(ICalendarMethod::Request)) => Method::Request,
        Some(ICalendarValue::Method(ICalendarMethod::Cancel)) => Method::Cancel,
        _ => Method::Other,
    }
}

/// A date or date-time property -- `DTSTART`, `DTEND`, `RECURRENCE-ID` --
/// read with its own `TZID`, and the zone it turned out to be in.
fn time(entry: &ICalendarEntry, zones: &TzResolver<&str>) -> Option<(EventTime, Zone)> {
    let value = entry.values.first()?.as_partial_date_time()?;
    let reading = value.to_date_time()?;
    if !value.has_time() {
        return Some((EventTime::Day(reading.date_time.date()), Zone::Floating));
    }
    if let Some(offset) = reading.offset {
        let instant =
            (reading.date_time - Duration::seconds(i64::from(offset.local_minus_utc()))).and_utc();
        let seconds = offset.local_minus_utc();
        let zone = if seconds == 0 {
            Zone::Utc
        } else {
            let sign = if seconds < 0 { '-' } else { '+' };
            let minutes = seconds.unsigned_abs() / 60;
            Zone::Named(format!("UTC{sign}{:02}:{:02}", minutes / 60, minutes % 60))
        };
        return Some((EventTime::At(instant), zone));
    }
    let Some(tzid) = entry.tz_id() else {
        return Some((EventTime::Floating(reading.date_time), Zone::Floating));
    };
    match zones.resolve(tzid).filter(|zone| !zone.is_floating()) {
        Some(zone) => Some((
            EventTime::At(local_instant(&zone, reading.date_time)),
            named(zone, tzid),
        )),
        None => Some((
            EventTime::Floating(reading.date_time),
            Zone::Unknown(tzid.to_owned()),
        )),
    }
}

/// A resolved zone as [`Zone`]: UTC, or its IANA name.
fn named(zone: Tz, tzid: &str) -> Zone {
    if zone.is_utc() {
        return Zone::Utc;
    }
    Zone::Named(zone.name().map_or_else(|| tzid.to_owned(), Cow::into_owned))
}

/// The event's `DURATION`.
fn duration(event: &ICalendarComponent) -> Option<Duration> {
    match event
        .property(&ICalendarProperty::Duration)?
        .values
        .first()?
    {
        ICalendarValue::Duration(duration) => duration.to_time_delta(),
        _ => None,
    }
}

/// `time` moved on by `length`. A whole day moves by whole days.
fn later(time: EventTime, length: Duration) -> EventTime {
    match time {
        EventTime::At(instant) => EventTime::At(instant + length),
        EventTime::Floating(local) => EventTime::Floating(local + length),
        EventTime::Day(day) => EventTime::Day(day + Duration::days(length.num_days())),
    }
}

/// RFC 5545 §3.6.1 with neither `DTEND` nor `DURATION`: a date-time event
/// ends where it starts, and a whole-day event lasts the day.
fn default_end(start: EventTime) -> EventTime {
    match start {
        EventTime::Day(day) => EventTime::Day(day + Duration::days(1)),
        other => other,
    }
}

/// `SEQUENCE`, `0` when absent or negative.
fn sequence(event: &ICalendarComponent) -> u32 {
    event
        .property(&ICalendarProperty::Sequence)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_integer)
        .map_or(0, |sequence| {
            u32::try_from(sequence.max(0)).unwrap_or(u32::MAX)
        })
}

/// `DTSTAMP` as an instant. RFC 5545 requires it in UTC; one written without
/// the `Z` is read as UTC anyway, since it could mean nothing else.
fn stamp(entry: &ICalendarEntry) -> Option<DateTime<Utc>> {
    let reading = entry
        .values
        .first()?
        .as_partial_date_time()?
        .to_date_time()?;
    let offset = reading
        .offset
        .map_or(0, |offset| i64::from(offset.local_minus_utc()));
    Some((reading.date_time - Duration::seconds(offset)).and_utc())
}

/// A text property, unescaped, or `None` when absent or blank.
fn text_of(component: &ICalendarComponent, property: &ICalendarProperty) -> Option<String> {
    component
        .property(property)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_text)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
}

/// The person an `ORGANIZER` or `ATTENDEE` names.
///
/// Its value is usually a `mailto:` URI. When it is some other URI -- a
/// `urn:uuid:`, a server's principal -- the `EMAIL` parameter (RFC 7986)
/// gives the address. Without either there is nobody to write to, and the
/// entry is left out. A `CN` that only repeats the address is no name.
fn address(entry: &ICalendarEntry) -> Option<EmailAddress> {
    let value = entry.values.first().and_then(ICalendarValue::as_text);
    let mailto = value.and_then(|value| {
        value
            .split_once(':')
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("mailto"))
            .map(|(_, address)| address.trim())
    });
    let address = mailto
        .or_else(|| {
            entry
                .parameter(&ICalendarParameterName::Email)
                .and_then(ICalendarParameterValue::as_text)
                .map(str::trim)
        })
        .filter(|address| address.contains('@'))?;
    let name = entry
        .parameter(&ICalendarParameterName::Cn)
        .and_then(ICalendarParameterValue::as_text)
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(address));
    Some(EmailAddress::new(name, address))
}

/// An attendee's `PARTSTAT`; absent means not answered (RFC 5545 §3.2.12).
fn partstat(entry: &ICalendarEntry) -> PartStat {
    match entry.parameter(&ICalendarParameterName::Partstat) {
        None => PartStat::NeedsAction,
        Some(ICalendarParameterValue::Partstat(status)) => match status {
            ICalendarParticipationStatus::NeedsAction => PartStat::NeedsAction,
            ICalendarParticipationStatus::Accepted => PartStat::Accepted,
            ICalendarParticipationStatus::Declined => PartStat::Declined,
            ICalendarParticipationStatus::Tentative => PartStat::Tentative,
            ICalendarParticipationStatus::Delegated => PartStat::Delegated,
            _ => PartStat::Other,
        },
        Some(_) => PartStat::Other,
    }
}

#[cfg(test)]
mod tests {
    use postio_model::EmailAddress;

    use super::*;
    use crate::invitation::{Attendee, Method, PartStat, Zone};
    use crate::test_support::{at, day, floating, ics, invitation, utc};

    fn named(zone: &str) -> Zone {
        Zone::Named(zone.to_owned())
    }

    fn person(name: Option<&str>, address: &str) -> EmailAddress {
        EmailAddress::new(name, address)
    }

    fn attendee(name: Option<&str>, address: &str, partstat: PartStat) -> Attendee {
        Attendee {
            address: person(name, address),
            partstat,
        }
    }

    /// Spike S1's table, now through the adapter: every invitation in the
    /// corpus, its start and end as instants, and the zone they were in.
    /// The times are chosen so that a wrong zone, or the right one at the
    /// wrong offset, gives a wrong instant.
    macro_rules! resolves {
        ($test:ident, $fixture:literal, $start:literal, $end:literal, $zone:expr) => {
            #[test]
            fn $test() {
                let invite = invitation($fixture);
                assert_eq!(
                    (invite.starts_at, invite.ends_at, invite.zone),
                    (at($start), at($end), $zone),
                    "{}: (start, end, zone)",
                    $fixture
                );
            }
        };
    }

    resolves!(
        a_windows_zone_name_is_europe_berlin_in_summer_time,
        "invite-windows-zone",
        "2026-10-06T08:00:00Z",
        "2026-10-06T08:45:00Z",
        named("Europe/Berlin")
    );
    resolves!(
        a_cancellation_resolves_the_same_times,
        "invite-cancel",
        "2026-10-06T08:00:00Z",
        "2026-10-06T08:45:00Z",
        named("Europe/Berlin")
    );
    resolves!(
        an_iana_zone_with_its_vtimezone_is_new_york_daylight_time,
        "invite-iana-zone",
        "2026-10-01T18:00:00Z",
        "2026-10-01T18:30:00Z",
        named("America/New_York")
    );
    resolves!(
        an_update_resolves_its_new_time,
        "invite-update-sequence",
        "2026-10-02T14:00:00Z",
        "2026-10-02T14:30:00Z",
        named("America/New_York")
    );
    resolves!(
        a_quoted_printable_part_is_london_on_gmt_after_the_clocks_go_back,
        "invite-quoted-printable",
        "2026-11-03T09:00:00Z",
        "2026-11-03T09:30:00Z",
        named("Europe/London")
    );
    resolves!(
        utc_times_stay_utc_whatever_a_stray_tzid_property_says,
        "invite-utc-times",
        "2026-10-15T16:00:00Z",
        "2026-10-15T17:00:00Z",
        Zone::Utc
    );
    resolves!(
        a_weekly_event_starts_on_its_first_occurrence,
        "invite-weekly-exdate",
        "2026-10-06T14:30:00Z",
        "2026-10-06T14:45:00Z",
        named("America/Chicago")
    );
    resolves!(
        a_tzid_with_no_vtimezone_resolves_by_name_at_a_half_hour_offset,
        "invite-zone-without-vtimezone",
        "2026-10-20T04:30:00Z",
        "2026-10-20T05:30:00Z",
        named("Asia/Kolkata")
    );
    resolves!(
        the_corpus_s_first_invite_is_stockholm_before_summer_time,
        "calendar-invite",
        "2026-03-12T13:00:00Z",
        "2026-03-12T14:00:00Z",
        named("Europe/Stockholm")
    );

    // --- Everything else an invitation says -----------------------------------

    #[test]
    fn a_request_carries_its_identity_people_and_place() {
        let invite = invitation("invite-windows-zone");

        assert_eq!(
            invite.uid,
            "040000008200E00074C5B7101A82E0080000000030C1F2A4B84FDC0100000000000000001000\
             0000A87C5E1B3D2F4A6B8C9D0E1F2A3B4C5D"
        );
        assert_eq!(invite.sequence, 0);
        assert_eq!(invite.stamp, Some(utc("2026-09-28T08:12:00Z")));
        assert_eq!(invite.method, Method::Request);
        assert!(!invite.cancelled);
        assert_eq!(invite.summary.as_deref(), Some("Q4 budget review"));
        assert_eq!(invite.location.as_deref(), Some("Room 4.12"));
        assert_eq!(
            invite.organizer,
            Some(person(Some("Ines Okafor"), "ines.okafor@example.com"))
        );
        assert_eq!(
            invite.attendees,
            vec![
                attendee(
                    Some("Ada Norwood"),
                    "ada.norwood@example.com",
                    PartStat::NeedsAction
                ),
                attendee(
                    Some("Quinn Abara"),
                    "quinn.abara@example.net",
                    PartStat::NeedsAction
                ),
            ]
        );
        assert!(!invite.recurring);
        assert_eq!(invite.recurrence_id, None);
    }

    #[test]
    fn an_alarm_s_attendee_is_not_a_guest_and_a_cn_that_is_the_address_is_no_name() {
        // Google lists the organiser as an attendee, names Ada by her address,
        // and puts a third ATTENDEE inside the VALARM.
        let invite = invitation("invite-iana-zone");

        assert_eq!(
            invite.attendees,
            vec![
                attendee(
                    Some("Mateo Varga"),
                    "mateo.varga@example.net",
                    PartStat::Accepted
                ),
                attendee(None, "ada.norwood@example.com", PartStat::NeedsAction),
            ]
        );
        assert_eq!(invite.location, None, "an empty LOCATION is no location");
    }

    #[test]
    fn a_quoted_printable_part_reads_its_quoted_parameters_and_the_event_s_own_uid() {
        let invite = invitation("invite-quoted-printable");

        assert_eq!(
            invite.uid, "5B2E8F1A-3C4D-4E6F-8A9B-0C1D2E3F4A5B",
            "not the VALARM's"
        );
        assert_eq!(
            invite.organizer,
            Some(person(
                Some("Priya Lindqvist"),
                "priya.lindqvist@example.org"
            ))
        );
        assert_eq!(
            invite.attendees,
            vec![
                attendee(
                    Some("Priya Lindqvist"),
                    "priya.lindqvist@example.org",
                    PartStat::Accepted
                ),
                attendee(
                    Some("Ada Norwood"),
                    "ada.norwood@example.com",
                    PartStat::NeedsAction
                ),
            ]
        );
        assert_eq!(invite.location.as_deref(), Some("Unit 7\nCanal Yard"));
    }

    #[test]
    fn an_attendee_with_no_partstat_has_not_answered() {
        let invite = invitation("invite-utc-times");

        assert_eq!(
            invite.attendees,
            vec![attendee(
                Some("Ada Norwood"),
                "ada.norwood@example.com",
                PartStat::NeedsAction
            )]
        );
        assert_eq!(
            invite.organizer,
            Some(person(Some("Hana Kowalczyk"), "hana.kowalczyk@example.org"))
        );
    }

    #[test]
    fn a_weekly_event_is_recurring() {
        assert!(invitation("invite-weekly-exdate").recurring);
        assert!(!invitation("invite-iana-zone").recurring);
    }

    #[test]
    fn an_update_keeps_the_uid_and_carries_its_sequence_and_stamp() {
        let original = invitation("invite-iana-zone");
        let update = invitation("invite-update-sequence");

        assert_eq!(update.uid, original.uid);
        assert_eq!(update.method, Method::Request);
        assert_eq!((original.sequence, update.sequence), (0, 1));
        assert_eq!(update.stamp, Some(utc("2026-09-28T09:15:02Z")));
    }

    #[test]
    fn a_cancellation_is_a_cancel_for_the_request_s_uid() {
        let request = invitation("invite-windows-zone");
        let cancel = invitation("invite-cancel");

        assert_eq!(cancel.method, Method::Cancel);
        assert!(cancel.cancelled);
        assert_eq!(cancel.uid, request.uid);
        assert_eq!(cancel.sequence, 1);
    }

    // --- Times the calendar does not anchor ------------------------------------

    fn event(lines: &[&str]) -> Vec<u8> {
        let mut all = vec![
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:edge-case@example.com",
            "DTSTAMP:20260927T120000Z",
        ];
        all.extend_from_slice(lines);
        all.extend_from_slice(&["END:VEVENT", "END:VCALENDAR"]);
        ics(&all)
    }

    #[test]
    fn a_floating_time_stays_floating_rather_than_becoming_utc() {
        let invite = parse(&event(&[
            "DTSTART:20261006T100000",
            "DTEND:20261006T110000",
        ]))
        .expect("parses");

        assert_eq!(invite.starts_at, floating("2026-10-06T10:00:00"));
        assert_eq!(invite.ends_at, floating("2026-10-06T11:00:00"));
        assert_eq!(invite.zone, Zone::Floating);
    }

    #[test]
    fn a_whole_day_with_no_end_lasts_one_day() {
        let invite = parse(&event(&["DTSTART;VALUE=DATE:20261015"])).expect("parses");

        assert_eq!(invite.starts_at, day("2026-10-15"));
        assert_eq!(invite.ends_at, day("2026-10-16"));
        assert_eq!(invite.zone, Zone::Floating);
    }

    #[test]
    fn a_duration_gives_the_end() {
        let invite =
            parse(&event(&["DTSTART:20261006T100000Z", "DURATION:PT45M"])).expect("parses");

        assert_eq!(invite.ends_at, at("2026-10-06T10:45:00Z"));
    }

    #[test]
    fn a_time_with_no_end_and_no_duration_ends_where_it_starts() {
        let invite = parse(&event(&["DTSTART:20261006T100000Z"])).expect("parses");

        assert_eq!(invite.ends_at, invite.starts_at);
    }

    #[test]
    fn a_tzid_nothing_resolves_keeps_its_name_and_its_wall_clock_times() {
        let invite = parse(&event(&[
            "DTSTART;TZID=Customized Time Zone:20261006T100000",
            "DTEND;TZID=Customized Time Zone:20261006T104500",
        ]))
        .expect("parses");

        assert_eq!(
            invite.zone,
            Zone::Unknown("Customized Time Zone".to_owned())
        );
        assert_eq!(invite.starts_at, floating("2026-10-06T10:00:00"));
        assert_eq!(invite.ends_at, floating("2026-10-06T10:45:00"));
    }

    #[test]
    fn a_time_written_with_a_numeric_offset_is_named_by_that_offset() {
        // Not RFC 5545, but written by hand-rolled generators. The instant is
        // exact; the zone is only the offset, half hour included.
        let invite = parse(&event(&["DTSTART:20261006T100000+0530"])).expect("parses");

        assert_eq!(invite.starts_at, at("2026-10-06T04:30:00Z"));
        assert_eq!(invite.zone, named("UTC+05:30"));
    }

    // --- Method and status -----------------------------------------------------

    #[test]
    fn a_request_whose_event_says_cancelled_is_cancelled() {
        let invite =
            parse(&event(&["DTSTART:20261006T100000Z", "STATUS:CANCELLED"])).expect("parses");

        assert_eq!(invite.method, Method::Request);
        assert!(invite.cancelled);
    }

    #[test]
    fn a_publish_or_a_calendar_with_no_method_is_other() {
        let published = ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "METHOD:PUBLISH",
            "BEGIN:VEVENT",
            "UID:published@example.com",
            "DTSTAMP:20260927T120000Z",
            "DTSTART:20261006T100000Z",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        let bare = ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "BEGIN:VEVENT",
            "UID:bare@example.com",
            "DTSTAMP:20260927T120000Z",
            "DTSTART:20261006T100000Z",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);

        assert_eq!(parse(&published).expect("parses").method, Method::Other);
        assert_eq!(parse(&bare).expect("parses").method, Method::Other);
    }

    // --- Series and occurrences --------------------------------------------------

    #[test]
    fn a_series_with_an_override_is_the_series() {
        let invite = parse(&ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:series@example.com",
            "DTSTAMP:20260927T120000Z",
            "RECURRENCE-ID:20261013T100000Z",
            "DTSTART:20261013T120000Z",
            "END:VEVENT",
            "BEGIN:VEVENT",
            "UID:series@example.com",
            "DTSTAMP:20260927T120000Z",
            "DTSTART:20261006T100000Z",
            "RRULE:FREQ=WEEKLY;COUNT=4",
            "END:VEVENT",
            "END:VCALENDAR",
        ]))
        .expect("parses");

        assert_eq!(invite.starts_at, at("2026-10-06T10:00:00Z"));
        assert!(invite.recurring);
        assert_eq!(invite.recurrence_id, None);
    }

    // --- When a series is over (T110) ---------------------------------------------

    #[test]
    fn a_series_with_an_end_is_over_when_its_last_occurrence_is() {
        // The weekly fixture runs on Tuesdays until 22 December at 09:30 in
        // Chicago, standard time by then, skipping two; each is a quarter of
        // an hour. An invitation to it is past only once that last one ends.
        let weekly = invitation("invite-weekly-exdate");

        assert_eq!(weekly.series_ends_at, Some(at("2026-12-22T15:45:00Z")));
        assert_eq!(weekly.last_end(), Some(at("2026-12-22T15:45:00Z")));
    }

    #[test]
    fn a_count_ends_a_series_too() {
        let daily = parse(&event(&[
            "DTSTART:20261006T100000Z",
            "DTEND:20261006T103000Z",
            "RRULE:FREQ=DAILY;COUNT=3",
        ]))
        .expect("parses");

        assert_eq!(daily.last_end(), Some(at("2026-10-08T10:30:00Z")));
    }

    #[test]
    fn a_floating_series_ends_on_the_reader_s_clock() {
        let floating_series = parse(&event(&[
            "DTSTART:20261006T100000",
            "DTEND:20261006T110000",
            "RRULE:FREQ=WEEKLY;COUNT=2",
        ]))
        .expect("parses");

        assert_eq!(
            floating_series.last_end(),
            Some(floating("2026-10-13T11:00:00"))
        );
    }

    #[test]
    fn a_series_with_no_end_is_never_over() {
        let forever = parse(&event(&[
            "DTSTART:20261006T100000Z",
            "RRULE:FREQ=WEEKLY;BYDAY=TU",
        ]))
        .expect("parses");

        assert_eq!(forever.series_ends_at, None);
        assert_eq!(forever.last_end(), None);
    }

    #[test]
    fn a_single_event_is_over_when_it_ends() {
        let single = invitation("invite-iana-zone");

        assert_eq!(single.series_ends_at, None);
        assert_eq!(single.last_end(), Some(single.ends_at));
    }

    #[test]
    fn an_update_to_one_occurrence_names_it() {
        let invite = parse(&event(&[
            "RECURRENCE-ID;TZID=America/Chicago:20261013T093000",
            "DTSTART;TZID=America/Chicago:20261013T110000",
            "DTEND;TZID=America/Chicago:20261013T111500",
        ]))
        .expect("parses");

        assert_eq!(invite.recurrence_id, Some(at("2026-10-13T14:30:00Z")));
        assert_eq!(invite.starts_at, at("2026-10-13T16:00:00Z"));
    }

    // --- Addresses -----------------------------------------------------------------

    #[test]
    fn an_attendee_named_by_a_uri_is_found_through_its_email_parameter() {
        let invite = parse(&event(&[
            "DTSTART:20261006T100000Z",
            "ATTENDEE;CN=Ada Norwood;EMAIL=ada.norwood@example.com;PARTSTAT=TENTATIVE:urn:uuid:0e1c2d3b-4a59-4687-9a8b-7c6d5e4f3a2b",
        ]))
        .expect("parses");

        assert_eq!(
            invite.attendees,
            vec![attendee(
                Some("Ada Norwood"),
                "ada.norwood@example.com",
                PartStat::Tentative
            )]
        );
    }

    #[test]
    fn an_attendee_with_no_address_at_all_is_left_out() {
        let invite = parse(&event(&[
            "DTSTART:20261006T100000Z",
            "ATTENDEE;CN=Conference Room 4:urn:uuid:5d6e7f80-91a2-4b3c-8d4e-5f6a7b8c9d0e",
        ]))
        .expect("parses");

        assert!(invite.attendees.is_empty());
    }

    // --- What yields no invitation ----------------------------------------------------

    #[test]
    fn text_that_is_not_a_calendar_is_refused() {
        assert_eq!(
            parse(b"Dear Ada, see you Tuesday."),
            Err(CalendarError::NotACalendar)
        );
        assert_eq!(parse(b""), Err(CalendarError::NotACalendar));
    }

    #[test]
    fn a_calendar_with_no_event_is_refused() {
        let todo = ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "BEGIN:VTODO",
            "UID:task@example.com",
            "DTSTAMP:20260927T120000Z",
            "END:VTODO",
            "END:VCALENDAR",
        ]);
        assert_eq!(parse(&todo), Err(CalendarError::NoEvent));
    }

    #[test]
    fn an_event_with_no_uid_or_no_start_is_refused() {
        let no_uid = ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Example Corp//Test//EN",
            "BEGIN:VEVENT",
            "DTSTAMP:20260927T120000Z",
            "DTSTART:20261006T100000Z",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        assert_eq!(parse(&no_uid), Err(CalendarError::NoUid));
        assert_eq!(
            parse(&event(&["SUMMARY:No time"])),
            Err(CalendarError::NoStart)
        );
    }

    #[test]
    fn a_stray_invalid_byte_does_not_cost_the_invitation() {
        let mut bytes = event(&["DTSTART:20261006T100000Z", "SUMMARY:Budget review"]);
        let at = bytes
            .windows(6)
            .position(|window| window == b"Budget")
            .expect("the summary");
        bytes[at] = 0xff;

        let invite = parse(&bytes).expect("parses");
        assert_eq!(invite.summary.as_deref(), Some("\u{fffd}udget review"));
    }
}
