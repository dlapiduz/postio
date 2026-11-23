use chrono::{TimeZone, Utc};
use postio_model::calendar::NewCalendar;
use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
use postio_storage::repository::{AccountRepository, CalendarRepository, EgressLogRepository};
use postio_storage::test_support;

#[tokio::test]
async fn a_new_store_has_on_this_computer_and_it_never_syncs() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");

    let calendars = CalendarRepository::new(&connection)
        .list()
        .await
        .expect("list");

    assert_eq!(calendars.len(), 1, "{calendars:?}");
    let local = &calendars[0];
    assert_eq!(local.name, "On this computer");
    assert_eq!(local.account, None);
    assert_eq!(local.href, None);
    assert!(local.writable && local.shown && !local.schedules);
    assert_eq!(
        CalendarRepository::new(&connection)
            .local()
            .await
            .expect("local")
            .id,
        local.id
    );
}

#[tokio::test]
async fn an_account_s_calendars_go_with_the_account() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let calendars = CalendarRepository::new(&connection);

    let work = calendars
        .create(&NewCalendar {
            account: Some(account.id),
            href: Some("/cal/ada/work/".to_owned()),
            name: "Work".to_owned(),
            colour: Some("#3584e4".to_owned()),
            writable: true,
            schedules: true,
        })
        .await
        .expect("create");
    let listed = calendars.list().await.expect("list");
    assert!(
        listed
            .iter()
            .any(|calendar| calendar.id == work && calendar.schedules)
    );

    AccountRepository::new(&connection)
        .delete(account.id)
        .await
        .expect("delete the account");

    let left: Vec<String> = calendars
        .list()
        .await
        .expect("list")
        .into_iter()
        .map(|calendar| calendar.name)
        .collect();
    assert_eq!(left, ["On this computer"]);
}

#[tokio::test]
async fn a_caldav_connection_is_a_row_in_the_egress_log() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let log = EgressLogRepository::new(&connection);

    log.record(&EgressEvent {
        at: Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap(),
        subsystem: EgressSubsystem::Caldav,
        account: None,
        host: "caldav.example.com".to_owned(),
        port: 443,
        outcome: EgressOutcome::Connected,
    })
    .await
    .expect("a caldav row is allowed");

    let recent = log.recent(1).await.expect("recent");
    assert_eq!(recent[0].subsystem, EgressSubsystem::Caldav);
}

#[tokio::test]
async fn a_change_keeps_what_it_replaced_after_its_event_is_gone() {
    use postio_model::calendar::EventStatus;
    use postio_storage::repository::{ChangeKind, StoredEvent};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let calendars = CalendarRepository::new(&connection);
    let local = calendars.local().await.expect("local").id;
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
    let event = calendars
        .put_event(&StoredEvent {
            calendar: local,
            uid: "dentist@example.com".to_owned(),
            href: None,
            etag: None,
            object: b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_vec(),
            title: "Dentist".to_owned(),
            location: String::new(),
            starts_at: at,
            recurs: false,
            status: EventStatus::Confirmed,
            my_partstat: None,
            source_message: None,
        })
        .await
        .expect("stored");
    let held = calendars.held(event).await.expect("read").expect("held");
    assert_eq!(
        (held.calendar, held.writable, held.recurs),
        (local, true, false)
    );

    let change = calendars
        .record_change(
            Some(event),
            local,
            ChangeKind::Delete,
            Some((&held.object, None)),
            Some("dentist.ics"),
            at,
        )
        .await
        .expect("recorded");
    calendars.delete_event(event).await.expect("deleted");

    let read = calendars.change(change).await.expect("read").expect("kept");
    assert_eq!(read.event, None, "the event is gone, the change is not");
    assert_eq!((read.calendar, read.kind), (local, ChangeKind::Delete));
    assert_eq!(read.base.as_deref(), Some(held.object.as_slice()));
    assert_eq!(read.href.as_deref(), Some("dentist.ics"));
    assert_eq!(calendars.held(event).await.expect("read"), None);
}
