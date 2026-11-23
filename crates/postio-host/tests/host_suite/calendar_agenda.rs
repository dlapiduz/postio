use chrono::{DateTime, Duration, FixedOffset, TimeZone, Utc};
use postio_host::calendar::{NewObject, agenda_page, store_object};
use postio_model::calendar::{AgendaAnchor, AgendaEntry};
use postio_model::ids::EventId;
use postio_storage::Store;
use postio_storage::repository::{CalendarRepository, StoredEvent, StoredOccurrence};
use postio_storage::test_support::{self, counting::counted_async};

const DST: &[u8] =
    include_bytes!("../../../postio-calendar/tests/fixtures/weekly-dst-exception.ics");
const FLOATING: &[u8] = include_bytes!("../../../postio-calendar/tests/fixtures/floating.ics");

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap()
}

fn daily(uid: &str) -> Vec<u8> {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Postio tests//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\n\
         DTSTAMP:20260101T000000Z\r\nDTSTART:20260101T090000Z\r\nDURATION:PT15M\r\n\
         RRULE:FREQ=DAILY\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
    .into_bytes()
}

async fn store(database: &Store, ics: &[u8]) -> EventId {
    let local = local_calendar(database).await;
    store_object(
        database,
        NewObject {
            calendar: local,
            ics,
            href: None,
            etag: None,
            my_partstat: None,
            source_message: None,
        },
        &Utc,
        now(),
    )
    .await
    .expect("stored")
}

async fn local_calendar(database: &Store) -> postio_model::ids::CalendarId {
    let connection = database.connect().await.expect("connection");
    CalendarRepository::new(&connection)
        .local()
        .await
        .expect("local")
        .id
}

async fn page(database: &Store, anchor: AgendaAnchor, limit: u32) -> Vec<AgendaEntry> {
    agenda_page(database, anchor, limit, &Utc, now())
        .await
        .expect("a page")
}

async fn with_occurrences(count: usize) -> Store {
    let database = test_support::memory().await;
    let calendar = local_calendar(&database).await;
    let connection = database.connect().await.expect("connection");
    let calendars = CalendarRepository::new(&connection);
    let event = calendars
        .put_event(&StoredEvent {
            calendar,
            uid: "many@example.com".to_owned(),
            href: None,
            etag: None,
            object: daily("many@example.com"),
            title: "Many".to_owned(),
            location: String::new(),
            starts_at: now(),
            recurs: false,
            status: postio_model::calendar::EventStatus::Confirmed,
            my_partstat: None,
            source_message: None,
        })
        .await
        .expect("event");
    let rows: Vec<StoredOccurrence> = (0..count)
        .map(|n| StoredOccurrence {
            start: now() + Duration::minutes(n as i64),
            end: now() + Duration::minutes(n as i64 + 1),
            all_day: false,
            floating: false,
            recurrence_id: None,
        })
        .collect();
    calendars.add_occurrences(event, &rows).await.expect("rows");
    database
}

#[tokio::test]
async fn a_page_costs_the_same_statements_at_fifty_thousand_occurrences() {
    let few = with_occurrences(60).await;
    let many = with_occurrences(50_000).await;
    let _ = page(&few, AgendaAnchor::From(now()), 50).await;
    let _ = page(&many, AgendaAnchor::From(now()), 50).await;

    let mut rows = 0;
    let small = counted_async(|| async {
        rows = page(&few, AgendaAnchor::From(now()), 50).await.len();
    })
    .await;
    let large = counted_async(|| async {
        rows = page(&many, AgendaAnchor::From(now()), 50).await.len();
    })
    .await;

    assert_eq!(rows, 50);
    assert_eq!(
        small.statements, large.statements,
        "a page grew with the store"
    );
    assert!(
        large.statements <= 3,
        "a page took {} statements",
        large.statements
    );
}

#[tokio::test]
async fn pages_after_and_before_a_row_continue_without_a_gap_or_a_repeat() {
    let database = with_occurrences(120).await;

    let first = page(&database, AgendaAnchor::From(now()), 50).await;
    let second = page(&database, AgendaAnchor::After(first[49].cursor()), 50).await;
    let back = page(&database, AgendaAnchor::Before(second[0].cursor()), 50).await;

    assert_eq!(second[0].start, first[49].start + Duration::minutes(1));
    assert_eq!(
        back, first,
        "paging back from the second page gives the first again"
    );
}

#[tokio::test]
async fn an_object_s_occurrences_land_at_their_instants_with_the_moved_one_named() {
    let database = test_support::memory().await;
    store(&database, DST).await;

    let rows = page(&database, AgendaAnchor::From(now()), 10).await;
    let starts: Vec<String> = rows
        .iter()
        .map(|row| row.start.format("%m-%d %H:%M").to_string())
        .collect();

    assert_eq!(
        starts,
        [
            "10-06 13:30",
            "10-13 13:30",
            "10-27 18:00",
            "11-03 14:30",
            "11-17 14:30",
            "11-24 14:30"
        ]
    );
    assert!(
        rows.iter()
            .all(|row| row.title == "Design sync" && row.calendar_name == "On this computer")
    );
    assert_eq!(rows[2].recurrence_id.as_deref(), Some("20261027T093000"));
}

#[tokio::test]
async fn a_floating_time_is_placed_on_the_local_clock_when_stored() {
    let database = test_support::memory().await;
    let calendar = local_calendar(&database).await;
    let new_york = FixedOffset::west_opt(5 * 3600).unwrap();
    store_object(
        &database,
        NewObject {
            calendar,
            ics: FLOATING,
            href: None,
            etag: None,
            my_partstat: None,
            source_message: None,
        },
        &new_york,
        now(),
    )
    .await
    .expect("stored");

    let rows = agenda_page(&database, AgendaAnchor::From(now()), 3, &new_york, now())
        .await
        .expect("a page");

    assert_eq!(
        rows[0].start,
        Utc.with_ymd_and_hms(2026, 12, 3, 12, 0, 0).unwrap()
    );
    assert!(rows[0].floating);
}

#[tokio::test]
async fn a_page_past_the_expanded_window_expands_the_rule_further() {
    let database = test_support::memory().await;
    store(&database, &daily("standup@example.com")).await;
    let later = now() + Duration::days(4 * 365);

    let rows = page(&database, AgendaAnchor::From(later), 3).await;

    assert_eq!(rows.len(), 3);
    assert!(rows[0].start >= later && rows[0].start < later + Duration::days(1));
    assert_eq!(rows[0].title, "Standup");
}

#[tokio::test]
async fn a_search_finds_events_by_their_words_and_bounds_and_names_what_it_ignored() {
    let database = test_support::memory().await;
    store(&database, DST).await;
    store(&database, FLOATING).await;

    let found = postio_host::calendar::search_events(&database, "design", &Utc, now(), 20)
        .await
        .expect("searched");
    assert_eq!(found.entries.len(), 1);
    assert_eq!(found.entries[0].title, "Design sync");
    assert_eq!(
        found.entries[0].start,
        Utc.with_ymd_and_hms(2026, 10, 6, 13, 30, 0).unwrap()
    );

    let found = postio_host::calendar::search_events(
        &database,
        "after:2026-11-15 with:ada.norwood@example.com is:unread",
        &Utc,
        now(),
        20,
    )
    .await
    .expect("searched");
    assert_eq!(found.entries.len(), 1);
    assert_eq!(
        found.entries[0].start,
        Utc.with_ymd_and_hms(2026, 11, 17, 14, 30, 0).unwrap()
    );
    assert_eq!(found.ignored, ["is"]);
}

#[tokio::test]
async fn the_calendar_window_s_state_is_remembered_without_a_position() {
    let database = test_support::memory().await;
    let state = postio_model::calendar::CalendarWindowState {
        popped: true,
        width: 900,
        height: 700,
    };

    assert_eq!(
        postio_host::calendar::calendar_window(&database)
            .await
            .expect("read"),
        Default::default(),
        "a new store has Calendar in the main window"
    );
    postio_host::calendar::set_calendar_window(&database, state)
        .await
        .expect("saved");
    assert_eq!(
        postio_host::calendar::calendar_window(&database)
            .await
            .expect("read"),
        state
    );
}
