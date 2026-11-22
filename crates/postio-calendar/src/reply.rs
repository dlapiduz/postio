use calcard::common::PartialDateTime;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarMethod,
    ICalendarParameter, ICalendarParticipationStatus, ICalendarProperty, ICalendarValue,
    ICalendarValueType, Uri,
};
use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};
use postio_model::EmailAddress;

use crate::invitation::{Answer, EventTime, Invitation};

/// Names the program and nothing about the person or the machine.
const PRODID: &str = "-//Postio//Postio//EN";

/// The bytes are the `text/calendar; method=REPLY` body, CRLF and folding included. Only the
/// replying
/// attendee is listed (RFC 5546 §3.2.3). Whether `attendee` may answer is the caller's decision;
/// when
/// the invitation lists the address, the reply spells it as the invitation does so the organiser
/// matches it.
pub fn reply(invitation: &Invitation, attendee: &EmailAddress, answer: Answer) -> Vec<u8> {
    reply_at(invitation, attendee, answer, Utc::now())
}

pub(crate) fn reply_at(
    invitation: &Invitation,
    attendee: &EmailAddress,
    answer: Answer,
    now: DateTime<Utc>,
) -> Vec<u8> {
    let mut root = ICalendarComponent::new(ICalendarComponentType::VCalendar);
    root.add_property(ICalendarProperty::Prodid, PRODID);
    root.add_property(ICalendarProperty::Version, "2.0");
    root.add_property(ICalendarProperty::Method, ICalendarMethod::Reply);
    root.component_ids = vec![1];

    let mut event = ICalendarComponent::new(ICalendarComponentType::VEvent);
    event.add_uid(&invitation.uid);
    event.add_sequence(i64::from(invitation.sequence));
    event.add_dtstamp(PartialDateTime::from_utc_timestamp(now.timestamp()));
    if let Some(organizer) = &invitation.organizer {
        event
            .entries
            .push(person(ICalendarProperty::Organizer, organizer, None));
    }
    // As the invitation spells the attendee, when it lists them: the
    // organiser's calendar matches its own attendee list, not ours.
    let listed = invitation
        .attendees
        .iter()
        .map(|listed| &listed.address)
        .find(|listed| listed.same_address(attendee));
    let partstat = match answer {
        Answer::Accept => ICalendarParticipationStatus::Accepted,
        Answer::Decline => ICalendarParticipationStatus::Declined,
    };
    event.entries.push(person(
        ICalendarProperty::Attendee,
        listed.unwrap_or(attendee),
        Some(partstat),
    ));
    if let Some(occurrence) = invitation.recurrence_id {
        event
            .entries
            .push(time(ICalendarProperty::RecurrenceId, occurrence));
    }
    if let Some(summary) = &invitation.summary {
        event.add_property(ICalendarProperty::Summary, summary.as_str());
    }
    event
        .entries
        .push(time(ICalendarProperty::Dtstart, invitation.starts_at));
    event
        .entries
        .push(time(ICalendarProperty::Dtend, invitation.ends_at));

    ICalendar {
        components: vec![root, event],
    }
    .to_string()
    .into_bytes()
}

fn person(
    property: ICalendarProperty,
    address: &EmailAddress,
    partstat: Option<ICalendarParticipationStatus>,
) -> ICalendarEntry {
    let name = address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    ICalendarEntry::new(property)
        .with_param_opt(partstat.map(ICalendarParameter::partstat))
        .with_param_opt(name.map(|name| ICalendarParameter::cn(name.to_owned())))
        .with_value(Uri::Location(format!("mailto:{}", address.address)))
}

/// An instant is written in UTC; floating stays floating and a whole day stays a date.
fn time(property: ICalendarProperty, time: EventTime) -> ICalendarEntry {
    match time {
        EventTime::At(instant) => ICalendarEntry::new(property)
            .with_value(PartialDateTime::from_utc_timestamp(instant.timestamp())),
        EventTime::Floating(local) => ICalendarEntry::new(property).with_value(floating(local)),
        EventTime::Day(day) => ICalendarEntry::new(property)
            .with_param(ICalendarParameter::value(ICalendarValueType::Date))
            .with_value(ICalendarValue::PartialDateTime(Box::new(PartialDateTime {
                year: u16::try_from(day.year()).ok(),
                month: u8::try_from(day.month()).ok(),
                day: u8::try_from(day.day()).ok(),
                ..PartialDateTime::default()
            }))),
    }
}

fn floating(local: NaiveDateTime) -> PartialDateTime {
    PartialDateTime {
        year: u16::try_from(local.year()).ok(),
        month: u8::try_from(local.month()).ok(),
        day: u8::try_from(local.day()).ok(),
        hour: u8::try_from(local.hour()).ok(),
        minute: u8::try_from(local.minute()).ok(),
        second: u8::try_from(local.second()).ok(),
        ..PartialDateTime::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invitation::{Attendee, Method, PartStat};
    use crate::parse;
    use crate::test_support::{at, invitation, unfolded, utc};

    fn ada() -> EmailAddress {
        EmailAddress::new(Some("Ada Norwood"), "ada.norwood@example.com")
    }

    fn noon() -> DateTime<Utc> {
        utc("2026-09-28T12:00:00Z")
    }

    fn line<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
        text.split("\r\n")
            .filter(|line| {
                line.strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with([':', ';']))
            })
            .collect()
    }

    #[test]
    fn accepting_writes_a_reply_with_the_attendee_accepted() {
        let invite = invitation("invite-windows-zone");
        let bytes = reply_at(&invite, &ada(), Answer::Accept, noon());
        let text = unfolded(&bytes);

        assert_eq!(line(&text, "METHOD"), vec!["METHOD:REPLY"], "{text}");
        assert_eq!(
            line(&text, "ATTENDEE"),
            vec![r#"ATTENDEE;PARTSTAT=ACCEPTED;CN="Ada Norwood":mailto:ada.norwood@example.com"#],
            "{text}"
        );
    }

    #[test]
    fn declining_writes_a_reply_with_the_attendee_declined() {
        let invite = invitation("invite-windows-zone");
        let bytes = reply_at(&invite, &ada(), Answer::Decline, noon());
        let text = unfolded(&bytes);

        assert_eq!(line(&text, "METHOD"), vec!["METHOD:REPLY"], "{text}");
        assert_eq!(
            line(&text, "ATTENDEE"),
            vec![r#"ATTENDEE;PARTSTAT=DECLINED;CN="Ada Norwood":mailto:ada.norwood@example.com"#],
            "{text}"
        );
    }

    #[test]
    fn a_reply_names_the_event_the_version_and_the_organiser_and_reads_back() {
        let invite = invitation("invite-update-sequence");
        let bytes = reply_at(&invite, &ada(), Answer::Accept, noon());

        let answered = parse(&bytes).expect("a reply is a calendar the adapter reads");
        assert_eq!(answered.uid, invite.uid);
        assert_eq!(answered.sequence, 1, "the version being answered");
        assert_eq!(answered.stamp, Some(noon()));
        assert_eq!(answered.method, Method::Other, "a REPLY asks nothing");
        assert_eq!(answered.organizer, invite.organizer);
        assert_eq!(answered.summary, invite.summary);
        assert_eq!(
            (answered.starts_at, answered.ends_at),
            (invite.starts_at, invite.ends_at)
        );
        assert_eq!(
            answered.attendees,
            vec![Attendee {
                address: EmailAddress::new(None::<String>, "ada.norwood@example.com"),
                partstat: PartStat::Accepted,
            }],
            "only the attendee who answers, as the invitation spelled her"
        );
    }

    #[test]
    fn a_reply_spells_the_attendee_as_the_invitation_does() {
        let invite = invitation("invite-iana-zone");
        let shouting = EmailAddress::new(Some("Ada N."), "ADA.NORWOOD@EXAMPLE.COM");
        let text = unfolded(&reply_at(&invite, &shouting, Answer::Accept, noon()));

        assert_eq!(
            line(&text, "ATTENDEE"),
            vec!["ATTENDEE;PARTSTAT=ACCEPTED:mailto:ada.norwood@example.com"],
            "{text}"
        );
    }

    #[test]
    fn an_attendee_the_invitation_does_not_list_answers_as_themselves() {
        let invite = invitation("invite-windows-zone");
        let other = EmailAddress::new(Some("Ada at home"), "ada@example.org");
        let text = unfolded(&reply_at(&invite, &other, Answer::Decline, noon()));

        assert_eq!(
            line(&text, "ATTENDEE"),
            vec![r#"ATTENDEE;PARTSTAT=DECLINED;CN="Ada at home":mailto:ada@example.org"#],
            "{text}"
        );
    }

    #[test]
    fn a_reply_to_one_occurrence_names_it() {
        let mut invite = invitation("invite-weekly-exdate");
        invite.recurrence_id = Some(at("2026-10-13T14:30:00Z"));
        let text = unfolded(&reply_at(&invite, &ada(), Answer::Accept, noon()));

        assert_eq!(
            line(&text, "RECURRENCE-ID"),
            vec!["RECURRENCE-ID:20261013T143000Z"],
            "{text}"
        );
        let answered = parse(text.as_bytes()).expect("reads back");
        assert_eq!(answered.recurrence_id, invite.recurrence_id);
    }

    #[test]
    fn a_reply_is_well_formed_icalendar() {
        let invite = invitation("invite-quoted-printable");
        let bytes = reply_at(&invite, &ada(), Answer::Accept, noon());
        let text = String::from_utf8(bytes).expect("UTF-8");

        assert!(text.starts_with("BEGIN:VCALENDAR\r\n"), "{text}");
        assert!(text.ends_with("END:VCALENDAR\r\n"), "{text}");
        for required in ["VERSION", "PRODID", "UID", "DTSTAMP", "ORGANIZER"] {
            assert_eq!(
                line(&unfolded(text.as_bytes()), required).len(),
                1,
                "exactly one {required}: {text}"
            );
        }
        for physical in text.split("\r\n") {
            assert!(physical.len() <= 75, "folded at 75 octets: {physical:?}");
            assert!(!physical.contains('\n'), "CRLF only: {physical:?}");
        }
    }

    #[test]
    fn the_clock_version_stamps_the_reply_now() {
        let invite = invitation("invite-windows-zone");
        let before = Utc::now() - chrono::Duration::seconds(1);
        let answered = parse(&reply(&invite, &ada(), Answer::Accept)).expect("reads back");
        let after = Utc::now() + chrono::Duration::seconds(1);

        let stamp = answered.stamp.expect("a DTSTAMP");
        assert!(before <= stamp && stamp <= after, "{stamp}");
    }
}
