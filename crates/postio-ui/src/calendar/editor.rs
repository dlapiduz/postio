use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_core::{CommandId, Keymap};
use postio_model::calendar::EditScope;

use crate::hints::{Hint, hint};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorField {
    Title,
    Starts,
    Ends,
    Location,
    People,
    Notes,
}

impl EditorField {
    pub const ALL: [EditorField; 6] = [
        EditorField::Title,
        EditorField::Starts,
        EditorField::Ends,
        EditorField::Location,
        EditorField::People,
        EditorField::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EditorField::Title => "Title",
            EditorField::Starts => "Starts",
            EditorField::Ends => "Ends",
            EditorField::Location => "Where",
            EditorField::People => "People",
            EditorField::Notes => "Notes",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            EditorField::Title => "What is it?",
            EditorField::Starts => "thu 3pm, 24 nov 09:30",
            EditorField::Ends => "1h, 90m, until 4",
            EditorField::Location => "A room or an address",
            EditorField::People => "Names or addresses, separated by commas",
            EditorField::Notes => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorView {
    pub heading: String,
    pub fields: Vec<(EditorField, String)>,
    pub when: Result<String, String>,
    pub scope: Option<EditScope>,
    pub error: Option<String>,
    pub keys: Vec<Hint>,
}

pub fn scope_label(scope: EditScope) -> &'static str {
    match scope {
        EditScope::This => "This event",
        EditScope::ThisAndFollowing => "This and following",
        EditScope::All => "All events",
    }
}

pub fn heading(editing: bool) -> &'static str {
    if editing { "Edit event" } else { "New event" }
}

pub fn start_text<Z: TimeZone>(at: DateTime<Utc>, zone: &Z) -> String
where
    Z::Offset: std::fmt::Display,
{
    at.with_timezone(zone).format("%-d %b %H:%M").to_string()
}

pub fn length_text(length: Duration) -> String {
    let minutes = length.num_minutes().max(0);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h{minutes:02}"),
    }
}

pub fn when_line<Z: TimeZone>(start: DateTime<Utc>, end: DateTime<Utc>, zone: &Z) -> String
where
    Z::Offset: std::fmt::Display,
{
    let (start, end) = (start.with_timezone(zone), end.with_timezone(zone));
    if start.date_naive() == end.date_naive() {
        format!(
            "{} \u{2013} {}",
            start.format("%a %-d %b, %H:%M"),
            end.format("%H:%M")
        )
    } else {
        format!(
            "{} \u{2013} {}",
            start.format("%a %-d %b, %H:%M"),
            end.format("%a %-d %b, %H:%M")
        )
    }
}

pub fn unread_start(typed: &str) -> String {
    format!("Postio cannot read \u{201c}{typed}\u{201d} as a start")
}

pub fn unread_end(typed: &str) -> String {
    format!("Postio cannot read \u{201c}{typed}\u{201d} as an end after the start")
}

pub const NO_TITLE: &str = "Give the event a title";

pub fn keys(keymap: &Keymap, inviting: bool) -> Vec<Hint> {
    let mut keys: Vec<Hint> = hint(keymap, CommandId::SaveEvent, "save")
        .into_iter()
        .collect();
    if inviting {
        keys.extend(hint(keymap, CommandId::SaveAndSendEvent, "save and invite"));
    }
    keys.extend(hint(keymap, CommandId::Back, "cancel"));
    keys
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn a_start_written_out_reads_back_as_itself() {
        let at = Utc.with_ymd_and_hms(2026, 11, 24, 15, 30, 0).unwrap();
        let text = start_text(at, &Utc);
        assert_eq!(text, "24 Nov 15:30");
        let now = Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap();
        assert_eq!(postio_search::date::parse_when(&text, now), Some(at));
    }

    #[test]
    fn a_length_is_written_the_way_it_is_typed() {
        assert_eq!(length_text(Duration::minutes(45)), "45m");
        assert_eq!(length_text(Duration::minutes(60)), "1h");
        assert_eq!(length_text(Duration::minutes(90)), "1h30");
        assert_eq!(length_text(Duration::minutes(125)), "2h05");
    }

    #[test]
    fn the_when_line_names_the_day_once_unless_it_runs_over() {
        let start = Utc.with_ymd_and_hms(2026, 11, 24, 15, 0, 0).unwrap();
        assert_eq!(
            when_line(start, start + Duration::minutes(90), &Utc),
            "Tue 24 Nov, 15:00 \u{2013} 16:30"
        );
        assert_eq!(
            when_line(start, start + Duration::hours(10), &Utc),
            "Tue 24 Nov, 15:00 \u{2013} Wed 25 Nov, 01:00"
        );
    }
}
