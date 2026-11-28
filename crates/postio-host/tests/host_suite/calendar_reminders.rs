use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_host::calendar::{NewObject, store_object};
use postio_session::reminders::{due_notices, next_due, plan_reminders, snooze};
use postio_storage::Store;
use postio_storage::repository::CalendarRepository;
use postio_storage::test_support;

const ALARMS: &[u8] = include_bytes!("../../../postio-calendar/tests/fixtures/email-alarm.ics");

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 12, hour, minute, 0).unwrap()
}

async fn with_dentist() -> Store {
    let database = test_support::memory().await;
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
            ics: ALARMS,
            href: None,
            etag: None,
            my_partstat: None,
            source_message: None,
        },
        &Utc,
        at(9, 0),
    )
    .await
    .expect("stored");
    plan_reminders(&database, at(9, 0), at(9, 0) + Duration::days(2))
        .await
        .expect("planned");
    database
}

#[tokio::test]
async fn display_and_audio_alarms_notify_and_the_email_alarm_does_not() {
    let database = with_dentist().await;
    let started = at(9, 0);

    assert!(
        due_notices(&database, at(14, 0), started)
            .await
            .expect("read")
            .is_empty(),
        "the email alarm an hour before sends nothing and shows nothing"
    );
    assert_eq!(next_due(&database).await.expect("read"), Some(at(14, 45)));
    let audio = due_notices(&database, at(14, 45), started)
        .await
        .expect("read");
    let display = due_notices(&database, at(14, 50), started)
        .await
        .expect("read");

    assert_eq!(audio.len(), 1);
    assert_eq!(display.len(), 1);
    assert_eq!(display[0].reminder.title, "Dentist");
    assert!(!display[0].missed);
    assert!(
        due_notices(&database, at(14, 55), started)
            .await
            .expect("read")
            .is_empty(),
        "once"
    );
}

#[tokio::test]
async fn snooze_brings_the_reminder_back_five_minutes_on() {
    let database = with_dentist().await;
    let fired = due_notices(&database, at(14, 50), at(9, 0))
        .await
        .expect("read");
    let display = fired.last().expect("a notice").reminder.clone();

    snooze(&database, &display, at(14, 50))
        .await
        .expect("snoozed");

    assert!(
        due_notices(&database, at(14, 54), at(9, 0))
            .await
            .expect("read")
            .is_empty()
    );
    let again = due_notices(&database, at(14, 55), at(9, 0))
        .await
        .expect("read");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].reminder.occurrence, display.occurrence);
}

#[tokio::test]
async fn a_reminder_missed_while_closed_shows_once_and_one_too_old_never_does() {
    let database = with_dentist().await;
    let started = at(16, 0);

    let caught_up = due_notices(&database, started, started)
        .await
        .expect("read");

    assert_eq!(caught_up.len(), 2, "both fell due while Postio was closed");
    assert!(caught_up.iter().all(|notice| notice.missed));
    assert!(
        due_notices(&database, at(16, 1), started)
            .await
            .expect("read")
            .is_empty()
    );

    let stale = with_dentist().await;
    let a_day_later = at(14, 50) + Duration::hours(13);
    assert!(
        due_notices(&stale, a_day_later, a_day_later)
            .await
            .expect("read")
            .is_empty(),
        "over twelve hours old is no help"
    );
    assert_eq!(next_due(&stale).await.expect("read"), None);
}
