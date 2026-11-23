use std::collections::BTreeSet;

use ical::component::IcalComponent;
use ical::ical::Ical;
use ical::param::IcalParam;
use ical::prop::{IcalProp, IcalPropKind, IcalPropName};
use ical::tree::cst::IcalCst;
use ical::tz::IcalTz;
use ical::value::IcalValue;

use crate::CalendarError;

/// A strict parse first; a calendar from the wild that only recovering reads is still read.
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

/// In first-seen order, so a master and its overrides expand together.
pub(crate) fn uids(calendar: &Ical<'_>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    events(calendar)
        .filter_map(|event| text(event, IcalPropKind::Uid))
        .filter(|uid| seen.insert(uid.clone()))
        .collect()
}

/// The object's own zones, completed from the bundled database, Windows names included.
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

/// A date or date-time as written, which is how `RECURRENCE-ID` names an instance.
pub(crate) fn raw_value(prop: &IcalProp<'_>) -> String {
    match &prop.value {
        IcalValue::DateTime(at) => at.0.trim().to_owned(),
        IcalValue::Date(day) => day.0.trim().to_owned(),
        _ => String::new(),
    }
}
