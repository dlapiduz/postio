use std::collections::BTreeSet;

use ical::component::IcalComponent;
use ical::ical::Ical;
use ical::param::IcalParam;
use ical::prop::{IcalProp, IcalPropKind, IcalPropName};
use ical::tree::cst::IcalCst;
use ical::tz::IcalTz;
use ical::value::IcalValue;

use crate::CalendarError;

pub(crate) fn parse(ics: &[u8]) -> Result<IcalCst<'_>, CalendarError> {
    if let Ok(cst) = IcalCst::parse(ics) {
        return Ok(cst);
    }
    IcalCst::parse_recovering(ics)
        .calendars
        .into_iter()
        .find(|cst| cst.decode().components.iter().any(is_event))
        .ok_or(CalendarError::NotACalendar)
}

pub(crate) fn is_event(component: &IcalComponent<'_>) -> bool {
    &*component.name == "VEVENT"
}

pub(crate) fn events<'c, 'a>(
    calendar: &'c Ical<'a>,
) -> impl Iterator<Item = &'c IcalComponent<'a>> {
    calendar
        .components
        .iter()
        .filter(|component| is_event(component))
}

pub(crate) fn uids(calendar: &Ical<'_>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    events(calendar)
        .filter_map(|event| text(event, IcalPropKind::Uid))
        .filter(|uid| seen.insert(uid.clone()))
        .collect()
}

pub(crate) fn zones(calendar: &Ical<'_>, through_year: i32) -> Vec<IcalTz> {
    ical::tzdb::zones(&calendar.components, through_year)
}

pub(crate) fn props<'c, 'a>(
    component: &'c IcalComponent<'a>,
    kind: IcalPropKind,
) -> impl Iterator<Item = &'c IcalProp<'a>> {
    component
        .props
        .iter()
        .filter(move |prop| matches!(prop.name, IcalPropName::Kind(found) if found == kind))
}

pub(crate) fn has(component: &IcalComponent<'_>, kind: IcalPropKind) -> bool {
    props(component, kind).next().is_some()
}

pub(crate) fn text(component: &IcalComponent<'_>, kind: IcalPropKind) -> Option<String> {
    props(component, kind).find_map(|prop| match &prop.value {
        IcalValue::Text(text) => Some(text.0.trim().to_owned()),
        _ => None,
    })
}

pub(crate) fn integer(component: &IcalComponent<'_>, kind: IcalPropKind) -> Option<i64> {
    props(component, kind).find_map(|prop| match &prop.value {
        IcalValue::Integer(number) => number.get(),
        _ => None,
    })
}

pub(crate) fn address(prop: &IcalProp<'_>) -> Option<String> {
    match &prop.value {
        IcalValue::CalAddress(address) => {
            let address = address.0.trim();
            let bare = address
                .get(..7)
                .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
                .map_or(address, |_| &address[7..]);
            (!bare.is_empty()).then(|| bare.to_owned())
        }
        _ => None,
    }
}

pub(crate) fn param<'p>(
    prop: &'p IcalProp<'_>,
    pick: fn(&'p IcalParam<'_>) -> Option<&'p str>,
) -> Option<&'p str> {
    prop.params.iter().find_map(pick)
}

pub(crate) fn raw_value(prop: &IcalProp<'_>) -> String {
    match &prop.value {
        IcalValue::DateTime(at) => at.0.trim().to_owned(),
        IcalValue::Date(day) => day.0.trim().to_owned(),
        _ => String::new(),
    }
}

fn civil(at: chrono::NaiveDateTime) -> ical::recur::IcalRecurDateTime {
    use chrono::{Datelike, Timelike};
    ical::recur::IcalRecurDateTime {
        year: at.year(),
        month: at.month() as u8,
        day: at.day() as u8,
        hour: at.hour() as u8,
        minute: at.minute() as u8,
        second: at.second() as u8,
    }
}

pub(crate) fn zone_local(
    tzid: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::NaiveDateTime> {
    use chrono::Datelike;
    let zone = ical::tzdb::zone(tzid, at.year() + 1)?;
    let local = zone.local(at.timestamp());
    chrono::NaiveDate::from_ymd_opt(local.year, u32::from(local.month), u32::from(local.day))?
        .and_hms_opt(
            u32::from(local.hour),
            u32::from(local.minute),
            u32::from(local.second),
        )
}

pub(crate) fn zone_instant(
    tzid: &str,
    at: chrono::NaiveDateTime,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::Datelike;
    let zone = ical::tzdb::zone(tzid, at.year() + 1)?;
    let wanted = civil(at);
    chrono::DateTime::from_timestamp(zone.resolve(wanted).instant(wanted)?, 0)
}
