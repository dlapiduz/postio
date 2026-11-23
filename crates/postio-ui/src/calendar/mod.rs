use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use postio_model::calendar::{
    AgendaCursor, AgendaEntry, EventDetail, EventStatus, Frequency, Participation, Repeat,
};
use postio_model::ids::EventId;

use crate::contacts_view::thousands;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgendaRow {
    Day { date: NaiveDate, label: String },
    Occurrence(OccurrenceRow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceRow {
    pub event: EventId,
    pub cursor: AgendaCursor,
    pub time: String,
    pub title: String,
    pub location: String,
    pub calendar_name: String,
    pub colour: Option<String>,
    pub chip: Option<Chip>,
    pub queued: bool,
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chip {
    Invite,
    Tentative,
    Declined,
    Clashes(String),
    Moved(String),
    Cancelled,
}

impl Chip {
    pub fn label(&self) -> String {
        match self {
            Chip::Invite => "Invite".to_owned(),
            Chip::Tentative => "Tentative".to_owned(),
            Chip::Declined => "Declined".to_owned(),
            Chip::Clashes(other) => format!("Clashes with {other}"),
            Chip::Moved(to) => format!("Moved to {to}"),
            Chip::Cancelled => "Cancelled".to_owned(),
        }
    }

    fn of(entry: &AgendaEntry) -> Option<Chip> {
        if entry.status == EventStatus::Cancelled {
            return Some(Chip::Cancelled);
        }
        match entry.my_partstat.as_deref() {
            Some("NEEDS-ACTION") => Some(Chip::Invite),
            Some("TENTATIVE") => Some(Chip::Tentative),
            Some("DECLINED") => Some(Chip::Declined),
            _ => None,
        }
    }
}

pub fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    let date = if day.year() == today.year() {
        day.format("%A %-d %B").to_string()
    } else {
        day.format("%A %-d %B %Y").to_string()
    };
    match (day - today).num_days() {
        0 => format!("Today \u{b7} {date}"),
        1 => format!("Tomorrow \u{b7} {date}"),
        _ => date,
    }
}

fn time_label<Z: TimeZone>(entry: &AgendaEntry, zone: &Z) -> String {
    if entry.all_day {
        return "All day".to_owned();
    }
    entry
        .start
        .with_timezone(zone)
        .naive_local()
        .format("%H:%M")
        .to_string()
}

fn local_day<Z: TimeZone>(at: DateTime<Utc>, zone: &Z) -> NaiveDate {
    at.with_timezone(zone).date_naive()
}

pub fn agenda_rows<Z: TimeZone>(
    entries: &[AgendaEntry],
    today: NaiveDate,
    zone: &Z,
) -> Vec<AgendaRow> {
    let mut rows = Vec::with_capacity(entries.len() + 4);
    let mut day = None;
    for entry in entries {
        let date = local_day(entry.start, zone);
        if day != Some(date) {
            day = Some(date);
            rows.push(AgendaRow::Day {
                date,
                label: day_label(date, today),
            });
        }
        let time = time_label(entry, zone);
        let mut accessible = vec![time.clone(), entry.title.clone()];
        if !entry.location.is_empty() {
            accessible.push(entry.location.clone());
        }
        accessible.push(calendar_words(&entry.calendar_name));
        rows.push(AgendaRow::Occurrence(OccurrenceRow {
            event: entry.event,
            cursor: entry.cursor(),
            time,
            title: entry.title.clone(),
            location: entry.location.clone(),
            calendar_name: entry.calendar_name.clone(),
            colour: entry.colour.clone(),
            chip: Chip::of(entry),
            queued: entry.queued,
            accessible: accessible.join(", "),
        }));
    }
    rows
}

fn calendar_words(name: &str) -> String {
    if name.to_lowercase().ends_with("calendar") || name == "On this computer" {
        name.to_owned()
    } else {
        format!("{name} calendar")
    }
}

pub fn empty_line(range: &str) -> String {
    format!("Nothing on {range}.")
}

pub fn no_account_line() -> &'static str {
    "Add a calendar account in Settings to sync"
}

pub fn conflict_line(server: &str) -> String {
    format!("Changed on {server} too")
}

pub fn keep_label() -> &'static str {
    "Keep this one"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarStrip {
    Synced,
    Syncing {
        calendar: String,
        done: u32,
        total: u32,
    },
    Offline {
        waiting: u32,
    },
    Failing {
        calendar: String,
        error: String,
    },
}

impl CalendarStrip {
    pub fn text(&self) -> String {
        match self {
            CalendarStrip::Synced => String::new(),
            CalendarStrip::Syncing {
                calendar,
                done,
                total,
            } => format!(
                "Syncing {} \u{b7} {} of {}",
                calendar_words(calendar),
                thousands(*done),
                thousands(*total)
            ),
            CalendarStrip::Offline { waiting } => format!(
                "Offline \u{b7} {waiting} {} waiting",
                if *waiting == 1 { "change" } else { "changes" }
            ),
            CalendarStrip::Failing { calendar, error } => {
                format!("{}: {error}", calendar_words(calendar))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventView {
    pub title: String,
    pub when: String,
    pub zone: Option<String>,
    pub repeat: Option<String>,
    pub location: String,
    pub people: Vec<String>,
    pub notes: String,
    pub calendar: String,
    pub reminders: Vec<String>,
}

pub fn event_view<Z: TimeZone>(detail: &EventDetail, zone: &Z, today: NaiveDate) -> EventView {
    let entry = &detail.entry;
    let day = local_day(entry.start, zone);
    let day_words = match (day - today).num_days() {
        0 => "Today".to_owned(),
        1 => "Tomorrow".to_owned(),
        _ => day.format("%a %-d %b").to_string(),
    };
    let when = if entry.all_day {
        format!("{day_words} \u{b7} All day")
    } else {
        format!(
            "{day_words} \u{b7} {}\u{2013}{}",
            entry
                .start
                .with_timezone(zone)
                .naive_local()
                .format("%H:%M"),
            entry.end.with_timezone(zone).naive_local().format("%H:%M")
        )
    };
    let mut people = Vec::new();
    if let Some(organiser) = &detail.organiser {
        people.push(format!("{}, organiser", person(organiser)));
    }
    for (attendee, answer) in &detail.attendees {
        if detail
            .organiser
            .as_ref()
            .is_some_and(|organiser| organiser.address == attendee.address)
        {
            continue;
        }
        people.push(format!("{}, {}", person(attendee), answer_words(*answer)));
    }
    EventView {
        title: entry.title.clone(),
        when,
        zone: detail.zone.clone(),
        repeat: detail.repeat.as_ref().map(repeat_words),
        location: entry.location.clone(),
        people,
        notes: detail.notes.clone(),
        calendar: calendar_words(&entry.calendar_name),
        reminders: detail
            .minutes_before
            .iter()
            .map(|minutes| reminder_words(*minutes))
            .collect(),
    }
}

fn person(address: &postio_model::EmailAddress) -> String {
    address
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.address.clone())
}

fn answer_words(answer: Participation) -> &'static str {
    match answer {
        Participation::NeedsAction => "not answered",
        Participation::Accepted => "accepted",
        Participation::Declined => "declined",
        Participation::Tentative => "tentative",
        Participation::Delegated => "delegated",
    }
}

fn reminder_words(minutes: i64) -> String {
    match minutes {
        0 => "At the start".to_owned(),
        1 => "1 minute before".to_owned(),
        m if m % (24 * 60) == 0 => plural(m / (24 * 60), "day"),
        m if m % 60 == 0 => plural(m / 60, "hour"),
        m => format!("{m} minutes before"),
    }
}

fn plural(n: i64, unit: &str) -> String {
    format!("{n} {unit}{} before", if n == 1 { "" } else { "s" })
}

const WEEKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

pub fn repeat_words(repeat: &Repeat) -> String {
    let unit = match repeat.frequency {
        Frequency::Daily => "day",
        Frequency::Weekly => "week",
        Frequency::Monthly => "month",
        Frequency::Yearly => "year",
    };
    let mut words = if repeat.frequency == Frequency::Weekly
        && repeat.interval <= 1
        && !repeat.weekdays.is_empty()
    {
        let mut days = repeat.weekdays.clone();
        days.sort_by_key(Weekday::num_days_from_monday);
        if days == WEEKDAYS {
            "Every weekday".to_owned()
        } else {
            let names: Vec<String> = days.iter().map(weekday_name).collect();
            format!("Every {}", join_and(&names))
        }
    } else if repeat.interval <= 1 {
        format!("Every {unit}")
    } else {
        format!("Every {} {unit}s", repeat.interval)
    };
    if let Some(until) = repeat.until {
        words.push_str(&format!(" until {}", until.format("%-d %b")));
    } else if let Some(count) = repeat.count {
        words.push_str(&format!(", {count} times"));
    }
    words
}

fn weekday_name(day: &Weekday) -> String {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
    .to_owned()
}

fn join_and(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
