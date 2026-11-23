use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_dav::{FakeCalendars, Fault};
use postio_model::egress::{EgressOutcome, EgressSubsystem};
use postio_model::ids::{AccountId, CalendarId};
use postio_storage::Store;
use postio_storage::repository::{CalendarRepository, ChangeKind, EgressLogRepository, HeldEvent};
use postio_storage::test_support;
use postio_sync::caldav::sync_account;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap()
}

fn event(uid: &str, title: &str, location: &str, start: DateTime<Utc>) -> Vec<u8> {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\n\
         DTSTAMP:20260101T000000Z\r\nDTSTART:{}\r\nDURATION:PT30M\r\nSUMMARY:{title}\r\n\
         LOCATION:{location}\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        start.format("%Y%m%dT%H%M%SZ")
    )
    .into_bytes()
}

struct World {
    database: Store,
    account: AccountId,
    fake: FakeCalendars,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await.id;
    drop(connection);
    let fake = FakeCalendars::new();
    fake.calendar("work", "Work", true)
        .calendar("holidays", "Holidays", false);
    fake.store(
        "work",
        "near.ics",
        &event(
            "near@example.com",
            "Standup",
            "Room 2",
            now() + Duration::days(1),
        ),
        Some(now() + Duration::days(1)),
    );
    fake.store(
        "work",
        "far.ics",
        &event(
            "far@example.com",
            "Old review",
            "",
            now() - Duration::days(900),
        ),
        Some(now() - Duration::days(900)),
    );
    World {
        database,
        account,
        fake,
    }
}

async fn sync(world: &World) -> postio_sync::caldav::Synced {
    sync_account(&world.database, &world.fake, world.account, now(), &Utc)
        .await
        .expect("synced")
}

async fn calendar(world: &World, href: &str) -> CalendarId {
    let connection = world.database.connect().await.expect("checkout");
    CalendarRepository::new(&connection)
        .remote(world.account)
        .await
        .expect("calendars")
        .into_iter()
        .find(|known| known.href == href)
        .expect("that calendar")
        .id
}

async fn held(world: &World, href: &str) -> Option<HeldEvent> {
    let work = calendar(world, "work").await;
    let connection = world.database.connect().await.expect("checkout");
    CalendarRepository::new(&connection)
        .held_at(work, href)
        .await
        .expect("a read")
}

async fn change_here(world: &World, href: &str, edits: postio_calendar::Edits) {
    let before = held(world, href).await.expect("held");
    let after = postio_calendar::patch(&before.object, &edits, &postio_calendar::Scope::All, &Utc)
        .expect("patched")
        .object;
    let connection = world.database.connect().await.expect("checkout");
    postio_sync::calendar::store_object_on(
        &connection,
        postio_sync::calendar::NewObject {
            calendar: before.calendar,
            ics: &after,
            href: before.href.clone(),
            etag: before.etag.clone(),
            my_partstat: None,
            source_message: None,
        },
        &Utc,
        now(),
    )
    .await
    .expect("stored");
    CalendarRepository::new(&connection)
        .record_change(
            Some(before.id),
            before.calendar,
            ChangeKind::Update,
            Some((&before.object, before.etag.as_deref())),
            before.href.as_deref(),
            now(),
        )
        .await
        .expect("recorded");
}

fn title(object: &[u8]) -> (String, String) {
    let event = &postio_calendar::parse_object(object).expect("parses")[0];
    (event.title.clone(), event.location.clone())
}

#[tokio::test]
async fn the_first_sync_takes_the_window_first_then_the_rest() {
    let world = world().await;

    let synced = sync(&world).await;

    assert_eq!(synced.fetched, 2);
    let requests = world.fake.requests();
    let windowed = requests
        .iter()
        .position(|r| r == "listing work windowed=true");
    let rest = requests
        .iter()
        .position(|r| r == "listing work windowed=false");
    assert!(windowed < rest && windowed.is_some(), "{requests:?}");
    assert!(held(&world, "near.ics").await.is_some());
    assert!(held(&world, "far.ics").await.is_some());
}

#[tokio::test]
async fn the_next_sync_asks_only_for_changes_since_its_token() {
    let world = world().await;
    sync(&world).await;
    world.fake.store(
        "work",
        "near.ics",
        &event(
            "near@example.com",
            "Standup (moved)",
            "Room 3",
            now() + Duration::days(1),
        ),
        Some(now() + Duration::days(1)),
    );
    world.fake.remove("work", "far.ics");
    let before = world.fake.requests().len();

    let synced = sync(&world).await;

    let requests = world.fake.requests()[before..].to_vec();
    assert!(
        !requests.iter().any(|r| r.starts_with("listing work")),
        "no listing once a token is held: {requests:?}"
    );
    assert_eq!((synced.fetched, synced.removed), (1, 1));
    let near = held(&world, "near.ics").await.expect("kept");
    assert_eq!(title(&near.object).0, "Standup (moved)");
    assert!(held(&world, "far.ics").await.is_none());
}

#[tokio::test]
async fn an_expired_token_starts_that_calendar_over() {
    let world = world().await;
    sync(&world).await;
    let work = calendar(&world, "work").await;
    {
        let connection = world.database.connect().await.expect("checkout");
        CalendarRepository::new(&connection)
            .set_synced(work, Some("a-token-the-server-forgot"), now())
            .await
            .expect("set");
    }

    sync(&world).await;

    let requests = world.fake.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| *r == "listing work windowed=true")
            .count(),
        2,
        "{requests:?}"
    );
    assert!(held(&world, "near.ics").await.is_some());
}

#[tokio::test]
async fn a_change_the_server_also_changed_merges_when_the_fields_differ() {
    let world = world().await;
    sync(&world).await;
    change_here(
        &world,
        "near.ics",
        postio_calendar::Edits {
            title: Some("Standup (short)".to_owned()),
            ..postio_calendar::Edits::default()
        },
    )
    .await;
    world.fake.store(
        "work",
        "near.ics",
        &event(
            "near@example.com",
            "Standup",
            "Room 9",
            now() + Duration::days(1),
        ),
        Some(now() + Duration::days(1)),
    );

    let synced = sync(&world).await;

    assert_eq!((synced.sent, synced.conflicts), (1, 0));
    let (_, server) = world
        .fake
        .object("work", "near.ics")
        .expect("on the server");
    assert_eq!(
        title(&server),
        ("Standup (short)".to_owned(), "Room 9".to_owned())
    );
    let here = held(&world, "near.ics").await.expect("here");
    assert_eq!(title(&here.object), title(&server));
}

#[tokio::test]
async fn the_same_field_changed_on_both_sides_is_a_conflict_and_nothing_is_lost() {
    let world = world().await;
    sync(&world).await;
    change_here(
        &world,
        "near.ics",
        postio_calendar::Edits {
            title: Some("Ours".to_owned()),
            ..postio_calendar::Edits::default()
        },
    )
    .await;
    world.fake.store(
        "work",
        "near.ics",
        &event(
            "near@example.com",
            "Theirs",
            "Room 2",
            now() + Duration::days(1),
        ),
        Some(now() + Duration::days(1)),
    );

    let synced = sync(&world).await;

    assert_eq!(synced.conflicts, 1);
    let (_, server) = world
        .fake
        .object("work", "near.ics")
        .expect("on the server");
    assert_eq!(title(&server).0, "Theirs");
    let here = held(&world, "near.ics").await.expect("here");
    assert_eq!(title(&here.object).0, "Ours");
}

#[tokio::test]
async fn a_refused_change_fails_and_the_calendar_reads_as_read_only() {
    let world = world().await;
    world.fake.store(
        "holidays",
        "new-year.ics",
        &event("ny@example.com", "New Year", "", now() + Duration::days(40)),
        Some(now() + Duration::days(40)),
    );
    sync(&world).await;
    let holidays = calendar(&world, "holidays").await;
    let before = {
        let connection = world.database.connect().await.expect("checkout");
        CalendarRepository::new(&connection)
            .held_at(holidays, "new-year.ics")
            .await
            .expect("a read")
            .expect("held")
    };
    {
        let connection = world.database.connect().await.expect("checkout");
        CalendarRepository::new(&connection)
            .record_change(
                Some(before.id),
                holidays,
                ChangeKind::Update,
                Some((&before.object, before.etag.as_deref())),
                before.href.as_deref(),
                now(),
            )
            .await
            .expect("recorded");
    }

    let synced = sync(&world).await;

    assert_eq!(synced.failed, 1);
    let connection = world.database.connect().await.expect("checkout");
    let remote = CalendarRepository::new(&connection)
        .remote(world.account)
        .await
        .expect("calendars");
    assert!(
        !remote
            .iter()
            .find(|known| known.id == holidays)
            .expect("held")
            .writable
    );
}

#[tokio::test]
async fn every_connection_is_a_caldav_row_in_the_egress_log() {
    let world = world().await;
    sync(&world).await;
    let unreachable = FakeCalendars::new();
    unreachable.fail_next(Fault::Unreachable);
    let failed = sync_account(&world.database, &unreachable, world.account, now(), &Utc).await;
    assert!(failed.is_err());

    let connection = world.database.connect().await.expect("checkout");
    let rows = EgressLogRepository::new(&connection)
        .recent(10)
        .await
        .expect("rows");
    let caldav: Vec<_> = rows
        .iter()
        .filter(|row| row.subsystem == EgressSubsystem::Caldav)
        .collect();
    assert_eq!(caldav.len(), 2, "{rows:?}");
    assert!(
        caldav
            .iter()
            .all(|row| row.host == "caldav.example.com" && row.account == Some(world.account))
    );
    assert!(
        caldav
            .iter()
            .any(|row| row.outcome == EgressOutcome::Failed)
    );
    assert!(
        caldav
            .iter()
            .any(|row| row.outcome == EgressOutcome::Connected)
    );
}
