use std::borrow::Cow;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ical::component::{IcalComponent, IcalComponentKind};
use ical::ical::Ical;
use ical::param::IcalParam;
use ical::prop::IcalProp;
use ical::tree::cst::{IcalCst, IcalItem};
use ical::tree::line::IcalLine;
use ical::tree::value::cursor::IcalValueCursor;
use ical::value::IcalValue;
use ical::value::cal_address::IcalCalAddress;
use ical::value::datetime::{IcalDate, IcalDateTime};
use ical::value::text::IcalText;
use ical::version::IcalVersion;
use postio_model::EmailAddress;

use crate::{CalendarError, Method};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Edits {
    pub title: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub shift: Option<Duration>,
    pub length: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    This(String),
    ThisAndFollowing(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub object: Vec<u8>,
    pub split: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub uid: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub location: String,
    pub notes: String,
    pub organiser: Option<EmailAddress>,
    pub attendees: Vec<EmailAddress>,
    pub alarm_minutes: Option<i64>,
}

fn parse_owned(ics: &[u8]) -> Result<IcalCst<'static>, CalendarError> {
    IcalCst::parse(ics)
        .map(IcalCst::into_static)
        .map_err(|_| CalendarError::NotACalendar)
}

fn is_event(cst: &IcalCst<'_>) -> bool {
    cst.begin
        .as_ref()
        .is_some_and(|begin| begin.raw_value_str().eq_ignore_ascii_case("VEVENT"))
}

fn lines<'c, 'a>(cst: &'c IcalCst<'a>, name: &'c str) -> impl Iterator<Item = &'c IcalLine<'a>> {
    cst.items.iter().filter_map(move |item| match item {
        IcalItem::Prop(line) if line.name.get().eq_ignore_ascii_case(name) => Some(line),
        _ => None,
    })
}

fn line_mut<'c, 'a>(cst: &'c mut IcalCst<'a>, name: &str) -> Option<&'c mut IcalLine<'a>> {
    cst.items.iter_mut().find_map(|item| match item {
        IcalItem::Prop(line) if line.name.get().eq_ignore_ascii_case(name) => Some(line),
        _ => None,
    })
}

fn rid_of(cst: &IcalCst<'_>) -> Option<String> {
    lines(cst, "RECURRENCE-ID")
        .next()
        .map(|line| line.raw_value_str().trim().to_owned())
}

fn drop_lines(cst: &mut IcalCst<'_>, names: &[&str]) {
    cst.items.retain(|item| match item {
        IcalItem::Prop(line) => !names
            .iter()
            .any(|name| line.name.get().eq_ignore_ascii_case(name)),
        _ => true,
    });
}

fn insert_line<'a>(cst: &mut IcalCst<'a>, line: IcalLine<'a>) {
    let at = cst
        .items
        .iter()
        .position(|item| matches!(item, IcalItem::Component(_)))
        .unwrap_or(cst.items.len());
    cst.items.insert(at, IcalItem::Prop(line));
}

fn set_text(line: &mut IcalLine<'_>, value: &str) {
    IcalValueCursor { line }.set_text(value);
}

fn set_or_add(cst: &mut IcalCst<'static>, name: &'static str, value: &str) {
    match line_mut(cst, name) {
        Some(line) => set_text(line, value),
        None => {
            let mut line = IcalLine::text(name, "");
            set_text(&mut line, value);
            insert_line(cst, line);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Civil {
    Day(NaiveDate),
    Local(NaiveDateTime),
    Utc(NaiveDateTime),
}

impl Civil {
    fn read(text: &str) -> Option<Civil> {
        let text = text.trim();
        if let Some(utc) = text.strip_suffix('Z') {
            return NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")
                .ok()
                .map(Civil::Utc);
        }
        if text.len() == 8 {
            return NaiveDate::parse_from_str(text, "%Y%m%d")
                .ok()
                .map(Civil::Day);
        }
        NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S")
            .ok()
            .map(Civil::Local)
    }

    fn write(self) -> String {
        match self {
            Civil::Day(day) => day.format("%Y%m%d").to_string(),
            Civil::Local(at) => at.format("%Y%m%dT%H%M%S").to_string(),
            Civil::Utc(at) => at.format("%Y%m%dT%H%M%SZ").to_string(),
        }
    }

    fn shifted(self, by: Duration) -> Civil {
        match self {
            Civil::Day(day) => Civil::Day(day + Duration::days(by.num_days())),
            Civil::Local(at) => Civil::Local(at + by),
            Civil::Utc(at) => Civil::Utc(at + by),
        }
    }
}

fn tzid(line: &IcalLine<'_>) -> Option<String> {
    line.params
        .iter()
        .find(|param| param.name.get().eq_ignore_ascii_case("TZID"))
        .and_then(|param| param.values.first())
        .map(|value| value.get().trim_matches('"').to_owned())
}

fn civil_for<Z: TimeZone>(line: &IcalLine<'_>, at: DateTime<Utc>, reader: &Z) -> Option<Civil> {
    let held = Civil::read(&line.raw_value_str())?;
    Some(match held {
        Civil::Day(_) => Civil::Day(at.with_timezone(reader).date_naive()),
        Civil::Utc(_) => Civil::Utc(at.naive_utc()),
        Civil::Local(_) => match tzid(line).and_then(|zone| crate::ics::zone_local(&zone, at)) {
            Some(local) => Civil::Local(local),
            None => Civil::Local(at.with_timezone(reader).naive_local()),
        },
    })
}

fn apply<Z: TimeZone>(event: &mut IcalCst<'static>, edits: &Edits, reader: &Z) {
    if let Some(title) = &edits.title {
        set_or_add(event, "SUMMARY", title);
    }
    if let Some(location) = &edits.location {
        set_or_add(event, "LOCATION", location);
    }
    if let Some(notes) = &edits.notes {
        set_or_add(event, "DESCRIPTION", notes);
    }
    for (name, at) in [("DTSTART", edits.start), ("DTEND", edits.end)] {
        let Some(at) = at else { continue };
        if let Some(line) = line_mut(event, name)
            && let Some(civil) = civil_for(line, at, reader)
        {
            set_text(line, &civil.write());
        }
    }
    if let Some(by) = edits.shift {
        for name in ["DTSTART", "DTEND"] {
            if let Some(line) = line_mut(event, name)
                && let Some(civil) = Civil::read(&line.raw_value_str())
            {
                set_text(line, &civil.shifted(by).write());
            }
        }
    }
    let first = lines(event, "DTSTART").next().cloned();
    if let Some(length) = edits.length
        && let Some(start) = first
        && let Some(civil) = Civil::read(&start.raw_value_str())
    {
        let end = civil.shifted(length).write();
        match line_mut(event, "DTEND") {
            Some(line) => set_text(line, &end),
            None => {
                drop_lines(event, &["DURATION"]);
                let mut line = start;
                line.name = ical::tree::leaf::IcalLeaf::from("DTEND");
                set_text(&mut line, &end);
                insert_line(event, line);
            }
        }
    }
    bump_sequence(event);
}

pub fn with_uid(ics: &[u8], uid: &str) -> Result<Vec<u8>, CalendarError> {
    let mut root = parse_owned(ics)?;
    drop_lines(&mut root, &["METHOD"]);
    for item in &mut root.items {
        if let IcalItem::Component(child) = item
            && is_event(child)
        {
            set_or_add(child, "UID", uid);
            set_or_add(child, "SEQUENCE", "0");
        }
    }
    Ok(root.to_bytes())
}

fn bump_sequence(event: &mut IcalCst<'static>) {
    let next = lines(event, "SEQUENCE")
        .next()
        .and_then(|line| line.raw_value_str().trim().parse::<u32>().ok())
        .map_or(1, |sequence| sequence.saturating_add(1));
    set_or_add(event, "SEQUENCE", &next.to_string());
}

fn shift_exceptions(root: &mut IcalCst<'static>, by: Duration) {
    for item in &mut root.items {
        let IcalItem::Component(child) = item else {
            continue;
        };
        if !is_event(child) {
            continue;
        }
        let overriding = rid_of(child).is_some();
        for item in &mut child.items {
            let IcalItem::Prop(line) = item else { continue };
            let name = line.name.get().to_ascii_uppercase();
            let moves = matches!(name.as_str(), "EXDATE" | "RDATE" | "RECURRENCE-ID")
                || (overriding && matches!(name.as_str(), "DTSTART" | "DTEND"));
            if !moves {
                continue;
            }
            let shifted: Option<Vec<String>> = line
                .raw_value_str()
                .split(',')
                .map(|value| Civil::read(value).map(|civil| civil.shifted(by).write()))
                .collect();
            if let Some(shifted) = shifted {
                set_text(line, &shifted.join(","));
            }
        }
    }
}

fn master_index(root: &IcalCst<'_>) -> Option<usize> {
    root.items.iter().position(|item| {
        matches!(item, IcalItem::Component(child) if is_event(child) && rid_of(child).is_none())
    })
}

fn component_mut<'c>(root: &'c mut IcalCst<'static>, index: usize) -> &'c mut IcalCst<'static> {
    match &mut root.items[index] {
        IcalItem::Component(child) => child,
        _ => unreachable!("an index master_index found"),
    }
}

fn length_of(master: &IcalCst<'_>) -> Option<Duration> {
    let start = Civil::read(&lines(master, "DTSTART").next()?.raw_value_str())?;
    let end = Civil::read(&lines(master, "DTEND").next()?.raw_value_str())?;
    Some(match (start, end) {
        (Civil::Day(a), Civil::Day(b)) => b - a,
        (Civil::Local(a), Civil::Local(b)) | (Civil::Utc(a), Civil::Utc(b)) => b - a,
        _ => return None,
    })
}

fn instance_of(master: &IcalCst<'static>, rid: &str) -> Option<IcalCst<'static>> {
    let mut instance = master.clone();
    drop_lines(&mut instance, &["RRULE", "RDATE", "EXDATE", "EXRULE"]);
    let start_line = lines(master, "DTSTART").next()?.clone();
    let start = Civil::read(rid)?;
    let mut recurrence = start_line.clone();
    recurrence.name = ical::tree::leaf::IcalLeaf::from("RECURRENCE-ID");
    set_text(&mut recurrence, rid);
    insert_line(&mut instance, recurrence);
    if let Some(line) = line_mut(&mut instance, "DTSTART") {
        set_text(line, &start.write());
    }
    if let (Some(length), Some(line)) = (length_of(master), line_mut(&mut instance, "DTEND")) {
        set_text(line, &start.shifted(length).write());
    }
    Some(instance)
}

pub fn patch<Z: TimeZone>(
    ics: &[u8],
    edits: &Edits,
    scope: &Scope,
    reader: &Z,
) -> Result<Edited, CalendarError> {
    let mut root = parse_owned(ics)?;
    let master = master_index(&root).ok_or(CalendarError::NoEvent)?;
    match scope {
        Scope::All => {
            apply(component_mut(&mut root, master), edits, reader);
            if let Some(by) = edits.shift {
                shift_exceptions(&mut root, by);
            }
            Ok(Edited {
                object: root.to_bytes(),
                split: None,
            })
        }
        Scope::This(rid) => {
            let existing = root.items.iter().position(|item| {
                matches!(item, IcalItem::Component(child)
                    if is_event(child) && rid_of(child).as_deref() == Some(rid.as_str()))
            });
            let at = match existing {
                Some(at) => at,
                None => {
                    let instance = instance_of(component_mut(&mut root, master), rid)
                        .ok_or(CalendarError::NoStart)?;
                    root.push_component(instance);
                    root.items.len() - 1
                }
            };
            apply(component_mut(&mut root, at), edits, reader);
            Ok(Edited {
                object: root.to_bytes(),
                split: None,
            })
        }
        Scope::ThisAndFollowing(rid) => {
            let template = component_mut(&mut root, master).clone();
            end_series_before(&mut root, master, rid)?;
            let mut next = IcalCst::v2();
            for item in &root.items {
                if let IcalItem::Prop(line) = item
                    && !line.name.get().eq_ignore_ascii_case("VERSION")
                {
                    next.items.push(IcalItem::Prop(line.clone()));
                }
            }
            let mut first = instance_of(&template, rid).ok_or(CalendarError::NoStart)?;
            drop_lines(&mut first, &["RECURRENCE-ID"]);
            let last = last_instance(ics);
            for line in lines(&template, "RRULE") {
                let mut rule = line.clone();
                let held = rule.raw_value_str().into_owned();
                let counted = held.to_ascii_uppercase().contains("COUNT=");
                let mut parts: Vec<String> = held
                    .split(';')
                    .filter(|part| !part.to_ascii_uppercase().starts_with("COUNT="))
                    .map(str::to_owned)
                    .collect();
                if counted && let Some(last) = last {
                    parts.push(format!("UNTIL={}", utc(last)));
                }
                let text = parts.join(";");
                set_text(&mut rule, &text);
                insert_line(&mut first, rule);
            }
            let uid = format!(
                "{}-{rid}",
                lines(&template, "UID")
                    .next()
                    .map(|line| line.raw_value_str().into_owned())
                    .unwrap_or_default()
            );
            set_or_add(&mut first, "UID", &uid);
            apply(&mut first, edits, reader);
            next.push_component(first);
            Ok(Edited {
                object: root.to_bytes(),
                split: Some(next.to_bytes()),
            })
        }
    }
}

fn last_instance(ics: &[u8]) -> Option<DateTime<Utc>> {
    let forever = DateTime::<Utc>::UNIX_EPOCH..DateTime::<Utc>::MAX_UTC;
    crate::expand(ics, forever)
        .ok()?
        .last()
        .map(|occurrence| occurrence.start)
}

fn end_series_before(
    root: &mut IcalCst<'static>,
    master: usize,
    rid: &str,
) -> Result<(), CalendarError> {
    let start_line = lines(component_mut(root, master), "DTSTART")
        .next()
        .cloned()
        .ok_or(CalendarError::NoStart)?;
    let until = match Civil::read(rid).ok_or(CalendarError::NoStart)? {
        Civil::Day(day) => Civil::Day(day - Duration::days(1)),
        Civil::Utc(at) => Civil::Utc(at - Duration::seconds(1)),
        Civil::Local(at) => {
            match tzid(&start_line).and_then(|zone| crate::ics::zone_instant(&zone, at)) {
                Some(instant) => Civil::Utc((instant - Duration::seconds(1)).naive_utc()),
                None => Civil::Local(at - Duration::seconds(1)),
            }
        }
    };
    let event = component_mut(root, master);
    if let Some(line) = line_mut(event, "RRULE") {
        let mut parts: Vec<String> = line
            .raw_value_str()
            .split(';')
            .filter(|part| {
                let upper = part.to_ascii_uppercase();
                !upper.starts_with("COUNT=") && !upper.starts_with("UNTIL=")
            })
            .map(str::to_owned)
            .collect();
        parts.push(format!("UNTIL={}", until.write()));
        set_text(line, &parts.join(";"));
    }
    bump_sequence(event);
    root.items.retain(|item| match item {
        IcalItem::Component(child) if is_event(child) => {
            rid_of(child).is_none_or(|other| other.as_str() < rid)
        }
        _ => true,
    });
    Ok(())
}

pub fn remove(ics: &[u8], scope: &Scope) -> Result<Option<Vec<u8>>, CalendarError> {
    let mut root = parse_owned(ics)?;
    let master = master_index(&root).ok_or(CalendarError::NoEvent)?;
    match scope {
        Scope::All => Ok(None),
        Scope::This(rid) => {
            root.items.retain(|item| match item {
                IcalItem::Component(child) if is_event(child) => {
                    rid_of(child).as_deref() != Some(rid.as_str())
                }
                _ => true,
            });
            let master = master_index(&root).ok_or(CalendarError::NoEvent)?;
            let event = component_mut(&mut root, master);
            let mut exdate = lines(event, "DTSTART")
                .next()
                .cloned()
                .ok_or(CalendarError::NoStart)?;
            exdate.name = ical::tree::leaf::IcalLeaf::from("EXDATE");
            set_text(&mut exdate, rid);
            insert_line(event, exdate);
            bump_sequence(event);
            Ok(Some(root.to_bytes()))
        }
        Scope::ThisAndFollowing(rid) => {
            end_series_before(&mut root, master, rid)?;
            Ok(Some(root.to_bytes()))
        }
    }
}

pub fn request(ics: &[u8], method: Method) -> Result<Vec<u8>, CalendarError> {
    let mut root = parse_owned(ics)?;
    drop_lines(&mut root, &["METHOD"]);
    let word = match method {
        Method::Cancel => "CANCEL",
        Method::Request | Method::Other => "REQUEST",
    };
    insert_line(&mut root, IcalLine::text("METHOD", word));
    if method == Method::Cancel {
        for item in &mut root.items {
            if let IcalItem::Component(child) = item
                && is_event(child)
            {
                set_or_add(child, "STATUS", "CANCELLED");
            }
        }
    }
    Ok(root.to_bytes())
}

fn text(name: &'static str, value: &str) -> IcalProp<'static> {
    IcalProp {
        name: name.into(),
        params: Vec::new(),
        value: IcalValue::Text(IcalText(Cow::Owned(value.to_owned()))),
    }
}

fn person(name: &'static str, address: &EmailAddress) -> IcalProp<'static> {
    let mut params = Vec::new();
    if let Some(cn) = address.name.as_deref().filter(|cn| !cn.trim().is_empty()) {
        params.push(IcalParam::Cn(Cow::Owned(cn.to_owned())));
    }
    if name == "ATTENDEE" {
        params.push(IcalParam::PartStat(Cow::Borrowed("NEEDS-ACTION")));
        params.push(IcalParam::Rsvp(Cow::Borrowed("TRUE")));
    }
    IcalProp {
        name: name.into(),
        params,
        value: IcalValue::CalAddress(IcalCalAddress(Cow::Owned(format!(
            "mailto:{}",
            address.address
        )))),
    }
}

fn utc(at: DateTime<Utc>) -> String {
    at.format("%Y%m%dT%H%M%SZ").to_string()
}

pub fn new_object<Z: TimeZone>(event: &NewEvent, stamp: DateTime<Utc>, reader: &Z) -> Vec<u8> {
    let mut props = vec![
        text("UID", &event.uid),
        IcalProp {
            name: "DTSTAMP".into(),
            params: Vec::new(),
            value: IcalValue::DateTime(IcalDateTime(Cow::Owned(utc(stamp)))),
        },
    ];
    let time = |name: &'static str, at: DateTime<Utc>| {
        if event.all_day {
            IcalProp {
                name: name.into(),
                params: vec![IcalParam::Value(Cow::Borrowed("DATE"))],
                value: IcalValue::Date(IcalDate(Cow::Owned(
                    at.with_timezone(reader)
                        .date_naive()
                        .format("%Y%m%d")
                        .to_string(),
                ))),
            }
        } else {
            IcalProp {
                name: name.into(),
                params: Vec::new(),
                value: IcalValue::DateTime(IcalDateTime(Cow::Owned(utc(at)))),
            }
        }
    };
    props.push(time("DTSTART", event.start));
    props.push(time("DTEND", event.end));
    props.push(text("SUMMARY", &event.title));
    if !event.location.is_empty() {
        props.push(text("LOCATION", &event.location));
    }
    if !event.notes.is_empty() {
        props.push(text("DESCRIPTION", &event.notes));
    }
    props.push(text("SEQUENCE", "0"));
    if let Some(organiser) = &event.organiser
        && !event.attendees.is_empty()
    {
        props.push(person("ORGANIZER", organiser));
    }
    props.extend(
        event
            .attendees
            .iter()
            .map(|attendee| person("ATTENDEE", attendee)),
    );
    let alarms = event
        .alarm_minutes
        .map(|minutes| IcalComponent {
            name: IcalComponentKind::VAlarm.into(),
            props: vec![
                text("ACTION", "DISPLAY"),
                text("DESCRIPTION", &event.title),
                IcalProp {
                    name: "TRIGGER".into(),
                    params: Vec::new(),
                    value: IcalValue::Duration(ical::value::duration::IcalDuration(Cow::Owned(
                        format!("-PT{minutes}M"),
                    ))),
                },
            ],
            components: Vec::new(),
        })
        .into_iter()
        .collect();
    let calendar = Ical {
        version: IcalVersion::V2_0,
        props: vec![text("PRODID", "-//Postio//Postio//EN")],
        components: vec![IcalComponent {
            name: IcalComponentKind::VEvent.into(),
            props,
            components: alarms,
        }],
    };
    calendar.to_string().into_bytes()
}

fn bookkeeping(action: &ical::tree::merge::IcalMergeAction<'_>) -> bool {
    use ical::tree::merge::IcalMergeAction as Action;
    let path = match action {
        Action::PropAdded { at, .. }
        | Action::PropRemoved { at, .. }
        | Action::ValueChanged { at, .. }
        | Action::ValueItemAdded { at, .. }
        | Action::ValueItemRemoved { at, .. }
        | Action::ParamAdded { at, .. }
        | Action::ParamRemoved { at, .. } => at,
        _ => return false,
    };
    ["SEQUENCE", "DTSTAMP", "LAST-MODIFIED"]
        .iter()
        .any(|name| path.name.eq_ignore_ascii_case(name))
}

fn highest_sequence(ics: &[u8]) -> u32 {
    String::from_utf8_lossy(ics)
        .lines()
        .filter_map(|line| line.strip_prefix("SEQUENCE:"))
        .filter_map(|value| value.trim().parse().ok())
        .max()
        .unwrap_or(0)
}

pub fn merge(base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<Option<Vec<u8>>, CalendarError> {
    let (base_cst, ours_cst, theirs_cst) =
        (parse_owned(base)?, parse_owned(ours)?, parse_owned(theirs)?);
    let report = ical::tree::merge::IcalMerge {
        base: &base_cst,
        left: &ours_cst,
        right: &theirs_cst,
    }
    .merge();
    let collided = report
        .conflicts
        .iter()
        .any(|conflict| match &conflict.left {
            ical::tree::merge::IcalMergeReason::Divergent(left) => {
                !(bookkeeping(left) && bookkeeping(&conflict.right))
            }
            ical::tree::merge::IcalMergeReason::Recurrence(_) => false,
        });
    if collided {
        return Ok(None);
    }
    let mut merged = report.merged.into_static();
    let next = highest_sequence(ours)
        .max(highest_sequence(theirs))
        .saturating_add(1);
    for item in &mut merged.items {
        if let IcalItem::Component(child) = item
            && is_event(child)
        {
            set_or_add(child, "SEQUENCE", &next.to_string());
        }
    }
    Ok(Some(merged.to_bytes()))
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeDelta};

    use super::*;
    use crate::test_support::utc as at;

    const DST: &[u8] = include_bytes!("../tests/fixtures/weekly-dst-exception.ics");
    const UNKNOWN: &[u8] = include_bytes!("../tests/fixtures/unknown-properties.ics");

    fn starts(ics: &[u8]) -> Vec<String> {
        crate::expand(ics, at("2026-09-01T00:00:00Z")..at("2027-01-01T00:00:00Z"))
            .expect("expands")
            .iter()
            .map(|occurrence| occurrence.start.format("%m-%d %H:%M").to_string())
            .collect()
    }

    fn reader() -> FixedOffset {
        FixedOffset::west_opt(5 * 3600).expect("offset")
    }

    #[test]
    fn an_edit_changes_the_field_and_keeps_every_property_it_does_not_model() {
        let edits = Edits {
            title: Some("Paint the hall, and the stairs".to_owned()),
            ..Edits::default()
        };
        let edited = patch(UNKNOWN, &edits, &Scope::All, &reader())
            .expect("patched")
            .object;
        let before = String::from_utf8_lossy(UNKNOWN);
        let after = String::from_utf8_lossy(&edited);

        let kept = |text: &str| -> Vec<String> {
            text.split("\r\n")
                .filter(|line| !line.starts_with("SUMMARY") && !line.starts_with("SEQUENCE"))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(kept(&before), kept(&after), "{after}");
        assert!(
            after.contains("SUMMARY:Paint the hall\\, and the stairs"),
            "{after}"
        );
        assert!(after.contains("BEGIN:VTODO") && after.contains("X-APPLE-SORT-ORDER:42"));
    }

    #[test]
    fn moving_every_occurrence_keeps_each_on_its_own_clock() {
        let edits = Edits {
            shift: Some(TimeDelta::days(7)),
            ..Edits::default()
        };
        let edited = patch(DST, &edits, &Scope::All, &reader())
            .expect("patched")
            .object;

        assert_eq!(starts(&edited)[0], "10-13 13:30");
        assert!(
            starts(&edited).contains(&"11-10 14:30".to_owned()),
            "{:?}",
            starts(&edited)
        );
    }

    #[test]
    fn a_new_length_moves_the_end_and_a_duration_becomes_an_end() {
        let daily = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VEVENT\r\nUID:d@example.com\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20261001T090000Z\r\nDURATION:PT15M\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let edits = Edits {
            length: Some(TimeDelta::minutes(45)),
            ..Edits::default()
        };
        let edited = patch(daily, &edits, &Scope::All, &reader())
            .expect("patched")
            .object;
        let text = String::from_utf8(edited.clone()).expect("utf-8");

        assert!(
            text.contains("DTEND:20261001T094500Z") && !text.contains("DURATION"),
            "{text}"
        );
        let occurrence = &crate::expand(
            &edited,
            at("2026-10-01T00:00:00Z")..at("2026-10-02T00:00:00Z"),
        )
        .expect("expands")[0];
        assert_eq!(occurrence.end - occurrence.start, TimeDelta::minutes(45));
    }

    #[test]
    fn a_duplicate_is_its_own_event_from_its_first_sequence() {
        let copy = with_uid(DST, "copy@example.com").expect("copied");
        let events = crate::parse_object(&copy).expect("parses");

        assert!(events.iter().all(|event| event.uid == "copy@example.com"));
        assert_eq!(starts(&copy), starts(DST));
        assert!(!String::from_utf8_lossy(&copy).contains("SEQUENCE:2"));
    }

    #[test]
    fn this_occurrence_alone_moves_as_an_exception() {
        let edits = Edits {
            shift: Some(TimeDelta::hours(1)),
            ..Edits::default()
        };
        let edited = patch(
            DST,
            &edits,
            &Scope::This("20261103T093000".to_owned()),
            &reader(),
        )
        .expect("patched")
        .object;

        assert_eq!(
            starts(&edited),
            [
                "10-06 13:30",
                "10-13 13:30",
                "10-27 18:00",
                "11-03 15:30",
                "11-17 14:30",
                "11-24 14:30"
            ]
        );
        assert_eq!(crate::parse_object(&edited).expect("parses").len(), 3);
    }

    #[test]
    fn an_existing_exception_is_edited_in_place() {
        let edits = Edits {
            title: Some("Design sync (late)".to_owned()),
            ..Edits::default()
        };
        let edited = patch(
            DST,
            &edits,
            &Scope::This("20261027T093000".to_owned()),
            &reader(),
        )
        .expect("patched")
        .object;
        let events = crate::parse_object(&edited).expect("parses");

        assert_eq!(events.len(), 2);
        assert_eq!(events[1].title, "Design sync (late)");
        assert_eq!(events[0].title, "Design sync");
    }

    #[test]
    fn this_and_following_ends_the_series_and_starts_another_that_keeps_its_end() {
        let edits = Edits {
            title: Some("Design sync (new team)".to_owned()),
            ..Edits::default()
        };
        let edited = patch(
            DST,
            &edits,
            &Scope::ThisAndFollowing("20261117T093000".to_owned()),
            &reader(),
        )
        .expect("patched");
        let split = edited.split.expect("a new series");

        assert_eq!(
            starts(&edited.object),
            ["10-06 13:30", "10-13 13:30", "10-27 18:00", "11-03 14:30"]
        );
        assert_eq!(starts(&split), ["11-17 14:30", "11-24 14:30"]);
        let new = &crate::parse_object(&split).expect("parses")[0];
        assert_eq!(new.title, "Design sync (new team)");
        assert_ne!(new.uid, "design-sync@example.com");
    }

    #[test]
    fn deleting_one_occurrence_excludes_it_and_deleting_the_rest_ends_the_series() {
        let one = remove(DST, &Scope::This("20261103T093000".to_owned()))
            .expect("removed")
            .expect("the rest stays");
        assert!(
            !starts(&one).contains(&"11-03 14:30".to_owned()),
            "{:?}",
            starts(&one)
        );
        assert_eq!(starts(&one).len(), 5);

        let rest = remove(DST, &Scope::ThisAndFollowing("20261117T093000".to_owned()))
            .expect("removed")
            .expect("the start stays");
        assert_eq!(
            starts(&rest),
            ["10-06 13:30", "10-13 13:30", "10-27 18:00", "11-03 14:30"]
        );

        assert_eq!(remove(DST, &Scope::All).expect("removed"), None);
    }

    #[test]
    fn a_request_and_a_cancellation_say_their_method_before_the_event() {
        let invite =
            String::from_utf8(request(UNKNOWN, Method::Request).expect("request")).expect("utf-8");
        let cancel =
            String::from_utf8(request(UNKNOWN, Method::Cancel).expect("cancel")).expect("utf-8");

        assert!(
            invite.find("METHOD:REQUEST").expect("a method")
                < invite.find("BEGIN:VEVENT").expect("an event")
        );
        assert!(
            cancel.contains("METHOD:CANCEL") && cancel.contains("STATUS:CANCELLED"),
            "{cancel}"
        );
    }

    #[test]
    fn a_new_event_reads_back_with_its_people_and_its_reminder() {
        let event = NewEvent {
            uid: "dentist-1@example.com".to_owned(),
            title: "Dentist, again".to_owned(),
            start: at("2026-11-26T15:00:00Z"),
            end: at("2026-11-26T15:30:00Z"),
            all_day: false,
            location: "Harbour Street".to_owned(),
            notes: "Bring the form.".to_owned(),
            organiser: Some(EmailAddress::new(
                Some("Ada Norwood"),
                "ada.norwood@example.com",
            )),
            attendees: vec![EmailAddress::new(None::<String>, "quinn@example.net")],
            alarm_minutes: Some(10),
        };

        let ics = new_object(&event, at("2026-11-23T09:00:00Z"), &reader());
        let read = &crate::parse_object(&ics).expect("parses")[0];

        assert_eq!(read.title, "Dentist, again");
        assert_eq!(read.location, "Harbour Street");
        assert_eq!(read.attendees.len(), 1);
        assert_eq!(
            read.organiser.as_ref().map(|who| who.address.as_str()),
            Some("ada.norwood@example.com")
        );
        assert_eq!(
            read.alarms[0].trigger,
            crate::Trigger::SecondsBeforeStart(600)
        );
        assert_eq!(starts(&ics), ["11-26 15:00"]);
    }

    #[test]
    fn edits_to_different_fields_merge_and_the_same_field_does_not() {
        let reader = reader();
        let ours = patch(
            UNKNOWN,
            &Edits {
                title: Some("Paint the hall".to_owned()),
                ..Edits::default()
            },
            &Scope::All,
            &reader,
        )
        .expect("ours")
        .object;
        let theirs = patch(
            UNKNOWN,
            &Edits {
                location: Some("Upstairs".to_owned()),
                ..Edits::default()
            },
            &Scope::All,
            &reader,
        )
        .expect("theirs")
        .object;

        let merged = merge(UNKNOWN, &ours, &theirs)
            .expect("parsed")
            .expect("no collision");
        let event = &crate::parse_object(&merged).expect("parses")[0];
        assert_eq!(
            (event.title.as_str(), event.location.as_str()),
            ("Paint the hall", "Upstairs")
        );
        assert!(
            String::from_utf8_lossy(&merged).contains("X-APPLE-SORT-ORDER:42"),
            "nothing else is lost"
        );
        assert!(highest_sequence(&merged) > highest_sequence(&ours));

        let rival = patch(
            UNKNOWN,
            &Edits {
                title: Some("Paint the stairs".to_owned()),
                ..Edits::default()
            },
            &Scope::All,
            &reader,
        )
        .expect("rival")
        .object;
        assert_eq!(merge(UNKNOWN, &ours, &rival).expect("parsed"), None);
    }
}
