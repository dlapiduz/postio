use chrono::{TimeZone, Utc};
use postio_host::calendar::{NewObject, agenda_page, event_detail, search_events, store_object};
use postio_model::calendar::AgendaAnchor;
use postio_storage::repository::{CalendarRepository, EgressLogRepository};
use postio_storage::test_support;

const HOSTILE: &[u8] = include_bytes!("../../../postio-calendar/tests/fixtures/hostile-urls.ics");

#[tokio::test]
async fn a_hostile_event_is_stored_listed_opened_and_found_with_no_connection() {
    let database = test_support::memory().await;
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();
    let local = {
        let connection = database.connect().await.expect("a connection");
        CalendarRepository::new(&connection)
            .local()
            .await
            .expect("local")
            .id
    };

    store_object(
        &database,
        NewObject {
            calendar: local,
            ics: HOSTILE,
            href: None,
            etag: None,
            my_partstat: None,
            source_message: None,
        },
        &Utc,
        now,
    )
    .await
    .expect("stored");
    let page = agenda_page(&database, AgendaAnchor::From(now), 10, &Utc, now)
        .await
        .expect("a page");
    let detail = event_detail(&database, page[0].cursor())
        .await
        .expect("a read")
        .expect("the event");
    let found = search_events(&database, "quarterly", &Utc, now, 10)
        .await
        .expect("searched");

    assert!(
        detail.notes.contains("https://meet.example.net/j/123"),
        "kept as text"
    );
    assert_eq!(found.entries.len(), 1);
    let connection = database.connect().await.expect("a connection");
    let rows = EgressLogRepository::new(&connection)
        .recent(10)
        .await
        .expect("rows");
    assert!(rows.is_empty(), "no URL in an event is fetched: {rows:?}");
}
