use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use postio_index::events::{EventDocument, ensure, index_event, search_events};
use postio_model::calendar::{EventStatus, NewCalendar};
use postio_model::ids::{CalendarId, EventId};
use postio_storage::Connection;
use postio_storage::repository::{CalendarRepository, StoredEvent, StoredOccurrence};
use postio_storage::test_support;

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 24).unwrap()
}

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
}

async fn event(
    connection: &Connection,
    calendar: CalendarId,
    uid: &str,
    start: DateTime<Utc>,
    document: EventDocument,
) -> EventId {
    let calendars = CalendarRepository::new(connection);
    let id = calendars
        .put_event(&StoredEvent {
            calendar,
            uid: uid.to_owned(),
            href: None,
            etag: None,
            object: Vec::new(),
            title: document.title.clone(),
            location: document.location.clone(),
            starts_at: start,
            recurs: false,
            status: EventStatus::Confirmed,
            my_partstat: None,
            source_message: None,
        })
        .await
        .expect("an event");
    calendars
        .set_occurrences(
            id,
            &[StoredOccurrence {
                start,
                end: start + Duration::minutes(30),
                all_day: false,
                floating: false,
                recurrence_id: None,
            }],
        )
        .await
        .expect("occurrence");
    index_event(connection, id, &document)
        .await
        .expect("indexed");
    id
}

fn document(title: &str, location: &str, people: &[&str]) -> EventDocument {
    EventDocument {
        title: title.to_owned(),
        location: location.to_owned(),
        notes: String::new(),
        people: people.iter().map(|person| (*person).to_owned()).collect(),
    }
}

async fn store() -> (postio_storage::Store, Vec<EventId>) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("connection");
    ensure(&connection).await.expect("the event index");
    let calendars = CalendarRepository::new(&connection);
    let local = calendars.local().await.expect("local").id;
    let work = calendars
        .create(&NewCalendar {
            account: None,
            href: Some("/work/".to_owned()),
            name: "Work".to_owned(),
            colour: None,
            writable: true,
            schedules: false,
        })
        .await
        .expect("work");
    let ids = vec![
        event(
            &connection,
            local,
            "a",
            at(20, 9),
            document("Dentist", "Harbour Street clinic", &[]),
        )
        .await,
        event(
            &connection,
            work,
            "b",
            at(25, 10),
            document(
                "Budget review",
                "Room 4.12",
                &["Ines Okafor <ines@example.com>"],
            ),
        )
        .await,
        event(
            &connection,
            work,
            "c",
            at(27, 14),
            document(
                "Café planning",
                "Corner café",
                &["Ada Norwood <ada@example.com>"],
            ),
        )
        .await,
    ];
    drop(connection);
    (database, ids)
}

async fn found(database: &postio_storage::Store, query: &str) -> (Vec<EventId>, Vec<String>) {
    let connection = database.connect().await.expect("connection");
    let hits = search_events(&connection, &postio_search::parse(query, day()), &Utc, 50)
        .await
        .expect("searched");
    (hits.events, hits.ignored)
}

#[tokio::test]
async fn free_text_matches_title_and_place_without_their_accents() {
    let (database, ids) = store().await;

    assert_eq!(found(&database, "dentist").await.0, [ids[0]]);
    assert_eq!(found(&database, "cafe").await.0, [ids[2]]);
    assert_eq!(found(&database, "clinic").await.0, [ids[0]]);
}

#[tokio::test]
async fn after_and_before_bound_the_occurrences() {
    let (database, ids) = store().await;

    assert_eq!(
        found(&database, "after:2026-11-24").await.0,
        [ids[1], ids[2]]
    );
    assert_eq!(
        found(&database, "before:2026-11-26").await.0,
        [ids[0], ids[1]]
    );
}

#[tokio::test]
async fn with_names_an_attendee_and_in_names_a_calendar() {
    let (database, ids) = store().await;

    assert_eq!(found(&database, "with:ines@example.com").await.0, [ids[1]]);
    assert_eq!(found(&database, "in:work").await.0, [ids[1], ids[2]]);
    assert_eq!(
        found(&database, "in:work -with:ada@example.com").await.0,
        [ids[1]]
    );
}

#[tokio::test]
async fn an_operator_with_no_event_meaning_narrows_nothing_and_says_so() {
    let (database, ids) = store().await;

    let (events, ignored) = found(&database, "has:attachment budget").await;

    assert_eq!(events, [ids[1]]);
    assert_eq!(ignored, ["has"]);
}
