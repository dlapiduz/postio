//! What a capture -- a task or a note written to the vault from a message --
//! is made from and what it says, with no toolkit in it.

use chrono::{Datelike, NaiveDate, Weekday};
use postio_model::MessageId;
use postio_model::listing::{MarkerKind, MarkerWhen};

use crate::focus_list::FocusRow;

/// The message a capture is made from, as the row or the open message says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The message, which the line's link opens.
    pub message: MessageId,
    /// Who sent it.
    pub sender: String,
    /// Its subject.
    pub subject: String,
    /// When it arrived, as a person reads it.
    pub when: String,
    /// The sentence its marker quotes, verbatim, when it has one.
    pub sentence: Option<String>,
    /// The day the mail says it is due, when it says.
    pub due: Option<NaiveDate>,
}

/// What a capture is made from: the row under the cursor when it is the
/// message aimed at, with its marker's sentence and day; otherwise only the
/// open message's title. `None` when the row is a digest, which is no one
/// message.
pub fn source(
    message: MessageId,
    cursor: Option<&FocusRow>,
    open_title: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<Source> {
    let Some(row) = cursor.filter(|row| row.id() == message) else {
        return Some(Source {
            message,
            sender: String::new(),
            subject: open_title.to_owned(),
            when: String::new(),
            sentence: None,
            due: None,
        });
    };
    let summary = &row.as_conversation()?.summary;
    let representative = &summary.representative;
    let marker = summary.marker.as_ref();
    Some(Source {
        message,
        sender: representative
            .from
            .as_ref()
            .map(|from| from.display().to_owned())
            .unwrap_or_default(),
        subject: representative.subject.clone().unwrap_or_default(),
        when: crate::row::timestamp(summary.last_at, now),
        sentence: marker.and_then(|marker| marker.excerpt.clone()),
        due: marker.and_then(|marker| match marker.when {
            Some(MarkerWhen::Due(at)) if marker.kind == MarkerKind::Todo => {
                Some(at.with_timezone(&chrono::Local).date_naive())
            }
            _ => None,
        }),
    })
}

/// What is said when `t` or `n` finds no vault to capture into.
pub const NO_VAULT: &str =
    "Name a vault under [focus.vault] in config.toml to capture tasks and notes";

/// A task or a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A line in the Obsidian Tasks format.
    Task,
    /// An entry appended to a note.
    Note,
}

impl Mode {
    /// The field's title.
    pub fn field(self) -> &'static str {
        match self {
            Mode::Task => "Task",
            Mode::Note => "Note",
        }
    }

    /// The write button's words.
    pub fn button(self) -> &'static str {
        match self {
            Mode::Task => "Add task",
            Mode::Note => "Add note",
        }
    }

    /// The preview's title.
    pub fn preview_title(self) -> &'static str {
        match self {
            Mode::Task => "This exact line will be appended",
            Mode::Note => "This exact entry will be appended",
        }
    }

    /// Whether the due row shows: only a task has a day.
    pub fn has_due(self) -> bool {
        self == Mode::Task
    }
}

/// What the project is called when there is none.
pub const INBOX: &str = "Inbox";

/// The first `weekday` after `today`, never `today` itself.
pub fn next_weekday(today: NaiveDate, weekday: Weekday) -> NaiveDate {
    let ahead = (7 + i64::from(weekday.num_days_from_monday())
        - i64::from(today.weekday().num_days_from_monday()))
        % 7;
    let ahead = if ahead == 0 { 7 } else { ahead };
    today + chrono::Duration::days(ahead)
}

/// One quick pick for the due day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// What it says: "Today", "Mon", "None".
    pub words: String,
    /// The day it chooses; `None` for no due day.
    pub day: Option<NaiveDate>,
}

/// The due day's quick picks, in day order: today, the coming Monday,
/// Wednesday and Friday, the day the mail itself names (`due`) among them
/// when it is none of those, and last "None".
pub fn quick_picks(today: NaiveDate, due: Option<NaiveDate>) -> Vec<Pick> {
    let mut days = vec![today];
    for weekday in [Weekday::Mon, Weekday::Wed, Weekday::Fri] {
        days.push(next_weekday(today, weekday));
    }
    if let Some(due) = due
        && !days.contains(&due)
    {
        days.push(due);
    }
    days.sort();
    let mut picks: Vec<Pick> = days
        .into_iter()
        .map(|day| Pick {
            words: if day == today {
                "Today".to_owned()
            } else {
                day.format("%a").to_string()
            },
            day: Some(day),
        })
        .collect();
    picks.push(Pick {
        words: "None".to_owned(),
        day: None,
    });
    picks
}

/// The chosen due day, written out: "Friday, 2 October 2026", or "No due
/// date".
pub fn due_label(due: Option<NaiveDate>) -> String {
    match due {
        Some(day) => day.format("%A, %-d %B %Y").to_string(),
        None => "No due date".to_owned(),
    }
}

/// The project's title: why it was suggested when the vault's suggestion is
/// the one on screen (`named_in_subject` is the project the subject names),
/// otherwise just "Project".
pub fn project_title(named_in_subject: Option<&str>) -> String {
    match named_in_subject {
        Some(name) => format!("Project \u{b7} suggested: the subject names {name}"),
        None => "Project".to_owned(),
    }
}

/// The tasks note's line when there is no project: "path (no project)".
pub fn inbox_note(tasks_note: &str) -> String {
    format!("{tasks_note} (no project)")
}

/// Whether a project called `name` is listed under the filter `wanted`.
pub fn project_listed(name: &str, wanted: &str) -> bool {
    name.to_lowercase().contains(&wanted.to_lowercase())
}

/// Whether the no-project Inbox is listed under the filter `wanted`.
pub fn inbox_listed(wanted: &str) -> bool {
    INBOX.to_lowercase().contains(&wanted.to_lowercase())
}

/// A project row's count: "3 open".
pub fn open_count(open: usize) -> String {
    format!("{open} open")
}

/// The line under the preview: where the capture goes, and when it is due.
pub fn footnote(mode: Mode, place: &str, due: Option<NaiveDate>) -> String {
    match mode {
        Mode::Task => format!(
            "Plain markdown, Obsidian Tasks format, written on this computer. It goes in {place}{}.",
            due.map(|day| format!(", due {}", day.format("%a")))
                .unwrap_or_default()
        ),
        Mode::Note => {
            format!("Plain markdown, written on this computer. It goes in {place}'s note.")
        }
    }
}

/// What capture says once its line is written: "Task added to Review ·
/// due Fri", "Note added to Inbox".
pub fn added(mode: Mode, place: &str, due: Option<NaiveDate>) -> String {
    match (mode, due) {
        (Mode::Task, Some(day)) => format!("Task added to {place} \u{b7} due {}", day.format("%a")),
        (Mode::Task, None) => format!("Task added to {place}"),
        (Mode::Note, _) => format!("Note added to {place}"),
    }
}

/// The capture window's line naming its message: "From Ada · Quarterly
/// review · Mon".
pub fn from_line(source: &Source) -> String {
    format!(
        "From {} \u{b7} {} \u{b7} {}",
        source.sender, source.subject, source.when
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_capture_says_once_written_names_its_place_and_its_day() {
        let friday = NaiveDate::from_ymd_opt(2026, 10, 2).expect("a day");
        assert_eq!(
            added(Mode::Task, "Review", Some(friday)),
            "Task added to Review \u{b7} due Fri"
        );
        assert_eq!(added(Mode::Task, INBOX, None), "Task added to Inbox");
        assert_eq!(
            added(Mode::Note, "Review", Some(friday)),
            "Note added to Review"
        );
    }

    #[test]
    fn the_from_line_names_the_sender_the_subject_and_when() {
        let source = Source {
            message: MessageId::new(42),
            sender: "Ada".to_owned(),
            subject: "Quarterly review".to_owned(),
            when: "Mon".to_owned(),
            sentence: None,
            due: None,
        };
        assert_eq!(
            from_line(&source),
            "From Ada \u{b7} Quarterly review \u{b7} Mon"
        );
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a day")
    }

    #[test]
    fn the_quick_picks_name_the_coming_weekday_never_today() {
        assert_eq!(
            next_weekday(day(2026, 9, 26), Weekday::Wed),
            day(2026, 9, 30)
        );
        assert_eq!(
            next_weekday(day(2026, 9, 26), Weekday::Sat),
            day(2026, 10, 3),
            "a week on, not today"
        );
    }

    #[test]
    fn the_mails_own_day_joins_the_picks_in_order_and_none_comes_last() {
        let saturday = day(2026, 9, 26);
        let words = |due| {
            quick_picks(saturday, due)
                .into_iter()
                .map(|pick| pick.words)
                .collect::<Vec<_>>()
        };
        assert_eq!(words(None), ["Today", "Mon", "Wed", "Fri", "None"]);
        assert_eq!(
            words(Some(day(2026, 9, 29))),
            ["Today", "Mon", "Tue", "Wed", "Fri", "None"]
        );
        assert_eq!(
            words(Some(day(2026, 9, 28))),
            ["Today", "Mon", "Wed", "Fri", "None"]
        );
    }

    #[test]
    fn the_sheets_words() {
        assert_eq!(due_label(None), "No due date");
        assert_eq!(due_label(Some(day(2026, 10, 2))), "Friday, 2 October 2026");
        assert_eq!(project_title(None), "Project");
        assert!(project_title(Some("Garden")).ends_with("the subject names Garden"));
        assert_eq!(open_count(3), "3 open");
        assert!(project_listed("Garden", "GAR"));
        assert!(inbox_listed("in"));
        assert!(!inbox_listed("zzz"));
        assert_eq!(
            footnote(Mode::Task, "Garden", Some(day(2026, 10, 2))),
            "Plain markdown, Obsidian Tasks format, written on this computer. It goes in Garden, due Fri."
        );
        assert_eq!(Mode::Note.button(), "Add note");
        assert!(!Mode::Note.has_due());
    }
}
