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

#[tokio::test]
async fn an_account_s_remote_calendars_keep_their_token_error_and_changes() {
    use postio_model::calendar::EventStatus;
    use postio_storage::repository::{ChangeKind, ChangeState, StoredEvent};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await.id;
    let calendars = CalendarRepository::new(&connection);
    let at = Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap();
    let remote = NewCalendar {
        account: Some(account),
        href: Some("work".to_owned()),
        name: "Work".to_owned(),
        colour: None,
        writable: true,
        schedules: true,
    };
    let work = calendars
        .upsert_remote(account, &remote)
        .await
        .expect("added");
    let again = calendars
        .upsert_remote(
            account,
            &NewCalendar {
                name: "Work (renamed)".to_owned(),
                ..remote
            },
        )
        .await
        .expect("updated");
    assert_eq!(work, again, "the same href is the same calendar");

    calendars
        .set_synced(work, Some("t1"), at)
        .await
        .expect("token");
    calendars.set_error(work, "forbidden").await.expect("error");
    calendars
        .set_writable(work, false)
        .await
        .expect("read-only");
    let listed = calendars.remote(account).await.expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].token.as_deref(), listed[0].writable),
        (Some("t1"), false)
    );

    let event = calendars
        .put_event(&StoredEvent {
            calendar: work,
            uid: "u@example.com".to_owned(),
            href: None,
            etag: None,
            object: b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_vec(),
            title: "Standup".to_owned(),
            location: String::new(),
            starts_at: at,
            recurs: false,
            status: EventStatus::Confirmed,
            my_partstat: None,
            source_message: None,
        })
        .await
        .expect("an event");
    calendars
        .set_remote(event, "u.ics", "e1")
        .await
        .expect("remote");
    assert_eq!(
        calendars
            .held_at(work, "u.ics")
            .await
            .expect("read")
            .map(|held| held.id),
        Some(event)
    );
    assert_eq!(
        calendars
            .held_uid(work, "u@example.com")
            .await
            .expect("read")
            .map(|held| held.id),
        Some(event)
    );
    assert_eq!(
        calendars.held_at(work, "none.ics").await.expect("read"),
        None
    );

    let change = calendars
        .record_change(
            Some(event),
            work,
            ChangeKind::Update,
            None,
            Some("u.ics"),
            at,
        )
        .await
        .expect("recorded");
    assert!(calendars.unsettled(event).await.expect("read"));
    assert_eq!(calendars.queued(work).await.expect("read").len(), 1);
    calendars
        .settle(change, ChangeState::Conflict)
        .await
        .expect("settled");
    assert!(calendars.queued(work).await.expect("read").is_empty());
    assert!(
        calendars.unsettled(event).await.expect("read"),
        "a conflict still waits"
    );
    calendars
        .settle(change, ChangeState::Sent)
        .await
        .expect("settled");
    assert!(!calendars.unsettled(event).await.expect("read"));
}

#[tokio::test]
async fn a_reminder_is_planned_once_fired_once_and_snoozed_forward() {
    use postio_model::calendar::EventStatus;
    use postio_storage::repository::{PlannedReminder, ReminderState, StoredEvent};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let calendars = CalendarRepository::new(&connection);
    let local = calendars.local().await.expect("local").id;
    let at = Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap();
    let event = calendars
        .put_event(&StoredEvent {
            calendar: local,
            uid: "r@example.com".to_owned(),
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
        .expect("an event");
    let due = at - chrono::Duration::minutes(10);
    let planned = PlannedReminder {
        event,
        occurrence: at,
        due,
    };
    calendars
        .plan_reminders(&[planned, planned])
        .await
        .expect("planned");

    assert_eq!(calendars.next_reminder().await.expect("read"), Some(due));
    assert!(
        calendars
            .pending_by(due - chrono::Duration::minutes(1), 10)
            .await
            .expect("read")
            .is_empty()
    );
    let pending = calendars.pending_by(due, 10).await.expect("read");
    assert_eq!(pending.len(), 1, "planned twice, held once");
    assert_eq!(pending[0].title, "Dentist");

    calendars
        .snooze_reminder(&pending[0], due + chrono::Duration::minutes(5))
        .await
        .expect("snoozed");
    assert_eq!(
        calendars.next_reminder().await.expect("read"),
        Some(due + chrono::Duration::minutes(5))
    );
    let again = calendars.pending_by(at, 10).await.expect("read");
    calendars
        .settle_reminder(&again[0], ReminderState::Fired)
        .await
        .expect("fired");
    assert_eq!(calendars.next_reminder().await.expect("read"), None);
}
