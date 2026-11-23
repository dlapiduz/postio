use chrono::{DateTime, Duration, Utc};
use postio_storage::repository::{CalendarRepository, StoredEvent, StoredOccurrence};

use crate::support::{self, Fixture};

async fn with_events(fixture: &Fixture) {
    let connection = fixture.database.connect().await.expect("a connection");
    let calendars = CalendarRepository::new(&connection);
    let local = calendars.local().await.expect("local").id;
    let now: DateTime<Utc> = postio_model::clock::now().to_utc();
    for (n, (title, from_now)) in [
        ("Earlier", -120),
        ("Next up", 60),
        ("After that", 180),
        ("Tomorrow's", 26 * 60),
    ]
    .into_iter()
    .enumerate()
    {
        let start = now + Duration::minutes(from_now);
        let event = calendars
            .put_event(&StoredEvent {
                calendar: local,
                uid: format!("case-{n}@example.test"),
                href: None,
                etag: None,
                object: format!(
                    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VEVENT\r\nUID:case-{n}@example.test\r\n\
                     DTSTAMP:20260101T000000Z\r\nDTSTART:{}\r\nDURATION:PT30M\r\nSUMMARY:{title}\r\n\
                     DESCRIPTION:Notes for {title}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                    start.format("%Y%m%dT%H%M%SZ")
                )
                .into_bytes(),
                title: title.to_owned(),
                location: String::new(),
                starts_at: start,
                recurs: false,
                status: postio_model::calendar::EventStatus::Confirmed,
                my_partstat: None,
                source_message: None,
            })
            .await
            .expect("an event");
        calendars
            .set_occurrences(
                event,
                &[StoredOccurrence {
                    start,
                    end: start + Duration::minutes(30),
                    all_day: false,
                    floating: false,
                    recurrence_id: None,
                }],
            )
            .await
            .expect("its occurrence");
    }
}

pub fn g_e_opens_agenda_on_the_next_event_and_return_and_escape_walk_back_home() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.test"), "Hello", "Hi.", 5)
            .await;
        with_events(&fixture).await;
        let (window, _client) = fixture.open().await;

        support::keys(&window, &["g", "e"]);
        assert!(
            crate::settle_until(async || window
                .calendar()
                .and_then(|view| view.ringed())
                .is_some_and(|row| row.title == "Next up"))
            .await,
            "Agenda opens with the ring on the next event that has not ended"
        );
        let view = window.calendar().expect("the agenda");

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || view
                .ringed()
                .is_some_and(|row| row.title == "After that"))
            .await,
            "j moves the ring to the next occurrence"
        );

        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || view
                .open_event()
                .is_some_and(|open| open.notes == "Notes for After that"))
            .await,
            "Return opens the event with its notes"
        );

        support::keys(&window, &["Escape"]);
        assert!(crate::settle_until(async || view.open_event().is_none()).await);
        assert_eq!(
            view.ringed().map(|row| row.title).as_deref(),
            Some("After that")
        );

        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.calendar().is_none()).await,
            "a second Escape leaves Calendar"
        );
        assert!(
            !support::subjects(&window).is_empty(),
            "the inbox is back with its rows"
        );
    });
}
