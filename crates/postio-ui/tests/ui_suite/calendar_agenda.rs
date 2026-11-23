use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use postio_model::EmailAddress;
use postio_model::calendar::{
    AgendaEntry, EventDetail, EventStatus, Frequency, Participation, Repeat,
};
use postio_model::ids::{CalendarId, EventId};
use postio_ui::calendar::{
    AgendaRow, CalendarStrip, Chip, agenda_rows, conflict_line, empty_line, event_view, keep_label,
    no_account_line,
};

fn at(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, minute, 0)
        .unwrap()
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 24).unwrap()
}

fn entry(id: i64, start: DateTime<Utc>, minutes: i64, title: &str) -> AgendaEntry {
    AgendaEntry {
        event: EventId::new(id),
        calendar: CalendarId::new(1),
        calendar_name: "Work".to_owned(),
        colour: Some("#3584e4".to_owned()),
        title: title.to_owned(),
        location: "Room 2".to_owned(),
        start,
        end: start + chrono::Duration::minutes(minutes),
        all_day: false,
        floating: false,
        recurrence_id: None,
        status: EventStatus::Confirmed,
        my_partstat: None,
        queued: false,
    }
}

fn occurrences(rows: &[AgendaRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            AgendaRow::Day { label, .. } => format!("# {label}"),
            AgendaRow::Occurrence(row) => format!("{} {}", row.time, row.title),
        })
        .collect()
}

#[test]
fn occurrences_are_grouped_under_their_local_day() {
    let entries = [
        entry(1, at(24, 9, 30), 15, "Standup"),
        entry(2, at(24, 14, 0), 60, "Review"),
        entry(3, at(25, 9, 30), 15, "Standup"),
        entry(4, at(27, 11, 0), 30, "Dentist"),
    ];

    assert_eq!(
        occurrences(&agenda_rows(&entries, today(), &Utc)),
        [
            "# Today \u{b7} Tuesday 24 November",
            "09:30 Standup",
            "14:00 Review",
            "# Tomorrow \u{b7} Wednesday 25 November",
            "09:30 Standup",
            "# Friday 27 November",
            "11:00 Dentist",
        ]
    );
}

#[test]
fn a_row_says_its_calendar_in_words_not_only_in_colour() {
    let rows = agenda_rows(&[entry(1, at(24, 9, 30), 15, "Standup")], today(), &Utc);
    let AgendaRow::Occurrence(row) = &rows[1] else {
        panic!("an occurrence row");
    };

    assert_eq!(row.accessible, "09:30, Standup, Room 2, Work calendar");
    assert_eq!(row.colour.as_deref(), Some("#3584e4"));
}

#[test]
fn an_all_day_event_says_all_day() {
    let mut offsite = entry(1, at(24, 0, 0), 3 * 24 * 60, "Offsite");
    offsite.all_day = true;

    let rows = agenda_rows(&[offsite], today(), &Utc);

    assert_eq!(occurrences(&rows)[1], "All day Offsite");
}

#[test]
fn the_answer_shows_as_a_chip_and_a_queued_change_as_a_mark() {
    let mut invited = entry(1, at(24, 9, 0), 30, "Kickoff");
    invited.my_partstat = Some("NEEDS-ACTION".to_owned());
    let mut maybe = entry(2, at(24, 10, 0), 30, "Lunch");
    maybe.my_partstat = Some("TENTATIVE".to_owned());
    let mut no = entry(3, at(24, 11, 0), 30, "Offsite");
    no.my_partstat = Some("DECLINED".to_owned());
    let mut cancelled = entry(4, at(24, 12, 0), 30, "Demo");
    cancelled.status = EventStatus::Cancelled;
    let mut waiting = entry(5, at(24, 13, 0), 30, "Dentist");
    waiting.queued = true;

    let chips: Vec<(Option<String>, bool)> =
        agenda_rows(&[invited, maybe, no, cancelled, waiting], today(), &Utc)
            .iter()
            .filter_map(|row| match row {
                AgendaRow::Occurrence(row) => {
                    Some((row.chip.as_ref().map(Chip::label), row.queued))
                }
                AgendaRow::Day { .. } => None,
            })
            .collect();

    assert_eq!(
        chips,
        [
            (Some("Invite".to_owned()), false),
            (Some("Tentative".to_owned()), false),
            (Some("Declined".to_owned()), false),
            (Some("Cancelled".to_owned()), false),
            (None, true),
        ]
    );
    assert_eq!(
        Chip::Clashes("Standup".to_owned()).label(),
        "Clashes with Standup"
    );
    assert_eq!(
        Chip::Moved("Wed 10:00".to_owned()).label(),
        "Moved to Wed 10:00"
    );
}

#[test]
fn every_state_has_its_words() {
    assert_eq!(empty_line("this week"), "Nothing on this week.");
    assert_eq!(
        no_account_line(),
        "Add a calendar account in Settings to sync"
    );
    assert_eq!(
        CalendarStrip::Syncing {
            calendar: "Work".to_owned(),
            done: 340,
            total: 1200
        }
        .text(),
        "Syncing Work calendar \u{b7} 340 of 1,200"
    );
    assert_eq!(
        CalendarStrip::Offline { waiting: 2 }.text(),
        "Offline \u{b7} 2 changes waiting"
    );
    assert_eq!(
        CalendarStrip::Offline { waiting: 1 }.text(),
        "Offline \u{b7} 1 change waiting"
    );
    assert_eq!(
        CalendarStrip::Failing {
            calendar: "Work".to_owned(),
            error: "the server refused the password".to_owned()
        }
        .text(),
        "Work calendar: the server refused the password"
    );
    assert_eq!(CalendarStrip::Synced.text(), "");
    assert_eq!(
        conflict_line("caldav.example.com"),
        "Changed on caldav.example.com too"
    );
    assert_eq!(keep_label(), "Keep this one");
}

#[test]
fn the_open_event_says_when_how_often_who_and_where() {
    let detail = EventDetail {
        entry: entry(1, at(24, 9, 30), 15, "Standup"),
        notes: "Blockers first.".to_owned(),
        organiser: Some(EmailAddress::new(
            Some("Ines Okafor"),
            "ines.okafor@example.com",
        )),
        attendees: vec![
            (
                EmailAddress::new(Some("Ada Norwood"), "ada.norwood@example.com"),
                Participation::Accepted,
            ),
            (
                EmailAddress::new(None::<String>, "quinn@example.net"),
                Participation::NeedsAction,
            ),
        ],
        zone: Some("America/New_York".to_owned()),
        repeat: Some(Repeat {
            frequency: Frequency::Weekly,
            interval: 1,
            weekdays: vec![
                chrono::Weekday::Mon,
                chrono::Weekday::Tue,
                chrono::Weekday::Wed,
                chrono::Weekday::Thu,
                chrono::Weekday::Fri,
            ],
            until: NaiveDate::from_ymd_opt(2026, 12, 18),
            count: None,
        }),
        minutes_before: vec![10],
        writable: true,
    };

    let view = event_view(&detail, &Utc, today());

    assert_eq!(view.title, "Standup");
    assert_eq!(view.when, "Today \u{b7} 09:30\u{2013}09:45");
    assert_eq!(view.zone.as_deref(), Some("America/New_York"));
    assert_eq!(view.repeat.as_deref(), Some("Every weekday until 18 Dec"));
    assert_eq!(view.calendar, "Work calendar");
    assert_eq!(
        view.people,
        [
            "Ines Okafor, organiser",
            "Ada Norwood, accepted",
            "quinn@example.net, not answered",
        ]
    );
    assert_eq!(view.reminders, ["10 minutes before"]);
}

#[test]
fn a_repeat_is_said_in_words() {
    let repeat = |frequency, interval, count| Repeat {
        frequency,
        interval,
        weekdays: Vec::new(),
        until: None,
        count,
    };
    let words = |repeat: Repeat| postio_ui::calendar::repeat_words(&repeat);

    assert_eq!(words(repeat(Frequency::Daily, 1, None)), "Every day");
    assert_eq!(words(repeat(Frequency::Weekly, 2, None)), "Every 2 weeks");
    assert_eq!(
        words(repeat(Frequency::Monthly, 1, Some(6))),
        "Every month, 6 times"
    );
    assert_eq!(words(repeat(Frequency::Yearly, 1, None)), "Every year");
    assert_eq!(
        words(Repeat {
            weekdays: vec![chrono::Weekday::Tue],
            ..repeat(Frequency::Weekly, 1, None)
        }),
        "Every Tuesday"
    );
}
