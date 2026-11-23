use chrono::{DateTime, NaiveDate, Utc, Weekday};
use ical::component::IcalComponent;
use ical::param::IcalParam;
use ical::prop::IcalPropKind;
use ical::recur::{IcalRecurFreq, IcalRecurRule, IcalRecurWeekday};
use ical::value::IcalValue;
use postio_model::EmailAddress;
use postio_model::calendar::{Frequency, Repeat};

use crate::{Attendee, CalendarError, PartStat, ics};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub recurrence_id: Option<String>,
    pub title: String,
    pub location: String,
    pub notes: String,
    pub organiser: Option<EmailAddress>,
    pub attendees: Vec<Attendee>,
    pub status: Status,
    pub sequence: u32,
    pub recurs: bool,
    pub zone: Option<String>,
    pub repeat: Option<Repeat>,
    pub alarms: Vec<Alarm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    pub action: AlarmAction,
    pub trigger: Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlarmAction {
    Display,
    Audio,
    Email,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    SecondsBeforeStart(i64),
    SecondsBeforeEnd(i64),
    At(DateTime<Utc>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Confirmed,
    Tentative,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Confirmed => "confirmed",
            Status::Tentative => "tentative",
            Status::Cancelled => "cancelled",
        }
    }
}

pub fn parse_object(ics: &[u8]) -> Result<Vec<Event>, CalendarError> {
    let cst = ics::parse(ics)?;
    let calendar = cst.decode();
    ics::events(&calendar).map(event).collect()
}

fn event(component: &IcalComponent<'_>) -> Result<Event, CalendarError> {
    let uid = ics::text(component, IcalPropKind::Uid)
        .filter(|uid| !uid.is_empty())
        .ok_or(CalendarError::NoUid)?;
    if !ics::has(component, IcalPropKind::DtStart) {
        return Err(CalendarError::NoStart);
    }
    let field = |kind| ics::text(component, kind).unwrap_or_default();
    Ok(Event {
        uid,
        recurrence_id: ics::props(component, IcalPropKind::RecurrenceId)
            .next()
            .map(ics::raw_value),
        title: field(IcalPropKind::Summary),
        location: field(IcalPropKind::Location),
        notes: field(IcalPropKind::Description),
        organiser: ics::props(component, IcalPropKind::Organizer)
            .next()
            .and_then(person),
        attendees: ics::props(component, IcalPropKind::Attendee)
            .filter_map(|prop| {
                Some(Attendee {
                    address: person(prop)?,
                    partstat: partstat(prop),
                })
            })
            .collect(),
        status: match field(IcalPropKind::Status).to_ascii_uppercase().as_str() {
            "CANCELLED" => Status::Cancelled,
            "TENTATIVE" => Status::Tentative,
            _ => Status::Confirmed,
        },
        sequence: ics::integer(component, IcalPropKind::Sequence)
            .and_then(|sequence| u32::try_from(sequence).ok())
            .unwrap_or(0),
        recurs: ics::has(component, IcalPropKind::RRule)
            || ics::has(component, IcalPropKind::RDate),
        zone: ics::props(component, IcalPropKind::DtStart)
            .next()
            .and_then(|prop| {
                ics::param(prop, |param| match param {
                    IcalParam::TzId(zone) => Some(zone.as_ref()),
                    _ => None,
                })
            })
            .map(str::to_owned),
        repeat: ics::props(component, IcalPropKind::RRule)
            .next()
            .and_then(|prop| match &prop.value {
                IcalValue::Recur(rule) => IcalRecurRule::parse(&rule.0).ok(),
                _ => None,
            })
            .and_then(|rule| repeat(&rule)),
        alarms: component
            .components
            .iter()
            .filter(|alarm| &*alarm.name == "VALARM")
            .filter_map(alarm)
            .collect(),
    })
}

fn repeat(rule: &IcalRecurRule) -> Option<Repeat> {
    let frequency = match rule.freq {
        IcalRecurFreq::Daily => Frequency::Daily,
        IcalRecurFreq::Weekly => Frequency::Weekly,
        IcalRecurFreq::Monthly => Frequency::Monthly,
        IcalRecurFreq::Yearly => Frequency::Yearly,
        _ => return None,
    };
    Some(Repeat {
        frequency,
        interval: rule.interval.max(1),
        weekdays: rule
            .by_day
            .iter()
            .filter(|day| day.ordinal.is_none())
            .map(|day| weekday(day.weekday))
            .collect(),
        until: rule.until.and_then(|until| {
            NaiveDate::from_ymd_opt(until.year, u32::from(until.month), u32::from(until.day))
        }),
        count: rule.count,
    })
}

fn weekday(day: IcalRecurWeekday) -> Weekday {
    match day {
        IcalRecurWeekday::Monday => Weekday::Mon,
        IcalRecurWeekday::Tuesday => Weekday::Tue,
        IcalRecurWeekday::Wednesday => Weekday::Wed,
        IcalRecurWeekday::Thursday => Weekday::Thu,
        IcalRecurWeekday::Friday => Weekday::Fri,
        IcalRecurWeekday::Saturday => Weekday::Sat,
        IcalRecurWeekday::Sunday => Weekday::Sun,
    }
}

fn alarm(component: &IcalComponent<'_>) -> Option<Alarm> {
    let action = match ics::text(component, IcalPropKind::Action)?
        .to_ascii_uppercase()
        .as_str()
    {
        "DISPLAY" => AlarmAction::Display,
        "AUDIO" => AlarmAction::Audio,
        "EMAIL" => AlarmAction::Email,
        _ => AlarmAction::Other,
    };
    let prop = ics::props(component, IcalPropKind::Trigger).next()?;
    let to_end = ics::param(prop, |param| match param {
        IcalParam::Related(related) => Some(related.as_ref()),
        _ => None,
    })
    .is_some_and(|related| related.eq_ignore_ascii_case("END"));
    let trigger = match &prop.value {
        IcalValue::Duration(duration) => {
            let before = -duration.seconds()?;
            if to_end {
                Trigger::SecondsBeforeEnd(before)
            } else {
                Trigger::SecondsBeforeStart(before)
            }
        }
        IcalValue::DateTime(at) => Trigger::At(utc_stamp(&at.0)?),
        _ => return None,
    };
    Some(Alarm { action, trigger })
}

fn utc_stamp(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim().trim_end_matches('Z');
    chrono::NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S")
        .ok()
        .map(|local| local.and_utc())
}

fn person(prop: &ical::prop::IcalProp<'_>) -> Option<EmailAddress> {
    let name = ics::param(prop, |param| match param {
        IcalParam::Cn(name) => Some(name.as_ref()),
        _ => None,
    });
    Some(EmailAddress::new(name, ics::address(prop)?))
}

fn partstat(prop: &ical::prop::IcalProp<'_>) -> PartStat {
    let status = ics::param(prop, |param| match param {
        IcalParam::PartStat(status) => Some(status.as_ref()),
        _ => None,
    });
    match status.map(str::to_ascii_uppercase).as_deref() {
        None | Some("NEEDS-ACTION") => PartStat::NeedsAction,
        Some("ACCEPTED") => PartStat::Accepted,
        Some("DECLINED") => PartStat::Declined,
        Some("TENTATIVE") => PartStat::Tentative,
        Some("DELEGATED") => PartStat::Delegated,
        Some(_) => PartStat::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DST: &[u8] = include_bytes!("../tests/fixtures/weekly-dst-exception.ics");
    const HOSTILE: &[u8] = include_bytes!("../tests/fixtures/hostile-urls.ics");
    const VTODO: &[u8] = include_bytes!("../tests/fixtures/unknown-properties.ics");

    #[test]
    fn a_series_reads_as_its_master_and_its_overrides() {
        let events = parse_object(DST).expect("parses");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].title, "Design sync");
        assert!(events[0].recurs);
        assert_eq!(events[0].recurrence_id, None);
        assert_eq!(events[0].sequence, 2);
        assert_eq!(events[1].title, "Design sync (afternoon)");
        assert_eq!(events[1].location, "Room 5");
        assert_eq!(events[1].recurrence_id.as_deref(), Some("20261027T093000"));
    }

    #[test]
    fn the_organiser_and_attendees_carry_names_and_answers() {
        let master = &parse_object(DST).expect("parses")[0];

        assert_eq!(
            master.organiser,
            Some(EmailAddress::new(
                Some("Ines Okafor"),
                "ines.okafor@example.com"
            ))
        );
        assert_eq!(
            master.attendees,
            [Attendee {
                address: EmailAddress::new(Some("Ada Norwood"), "ada.norwood@example.com"),
                partstat: PartStat::Accepted,
            }]
        );
        assert_eq!(master.status, Status::Confirmed);
    }

    #[test]
    fn a_url_in_an_event_is_only_text() {
        let event = &parse_object(HOSTILE).expect("parses")[0];

        assert!(
            event
                .notes
                .starts_with("Join at https://meet.example.net/j/123")
        );
        assert_eq!(event.location, "https://maps.example.net/?q=somewhere");
    }

    #[test]
    fn a_task_beside_an_event_is_not_read_as_one() {
        let events = parse_object(VTODO).expect("parses");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Paint the hallway");
    }

    #[test]
    fn alarms_are_read_with_their_action_and_trigger() {
        let event =
            &parse_object(include_bytes!("../tests/fixtures/email-alarm.ics")).expect("parses")[0];

        assert_eq!(
            event.alarms,
            [
                Alarm {
                    action: AlarmAction::Email,
                    trigger: Trigger::SecondsBeforeStart(3600)
                },
                Alarm {
                    action: AlarmAction::Display,
                    trigger: Trigger::SecondsBeforeStart(600)
                },
                Alarm {
                    action: AlarmAction::Audio,
                    trigger: Trigger::At(crate::test_support::utc("2026-11-12T14:45:00Z")),
                },
            ]
        );
    }

    #[test]
    fn a_rule_is_read_as_a_repeat_and_the_start_names_its_zone() {
        let master = &parse_object(DST).expect("parses")[0];

        assert_eq!(master.zone.as_deref(), Some("America/New_York"));
        assert_eq!(
            master.repeat,
            Some(Repeat {
                frequency: Frequency::Weekly,
                interval: 1,
                weekdays: vec![Weekday::Tue],
                until: None,
                count: Some(8),
            })
        );
    }

    #[test]
    fn an_event_without_a_uid_is_refused() {
        let ics = crate::test_support::ics(&[
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "PRODID:-//Postio tests//EN",
            "BEGIN:VEVENT",
            "DTSTAMP:20260101T000000Z",
            "DTSTART:20260101T090000Z",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);

        assert_eq!(parse_object(&ics), Err(CalendarError::NoUid));
    }
}
