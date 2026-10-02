//! A row's timestamp is a function of the message *and of now*, and now
//! belongs to `postio_ui::clock` so a storyboard can stop it. A row that
//! reads `Local::now()` itself films differently every day.
//!
//! Needs no display: the row's accessible label is built from the same
//! timestamp the column draws.

use chrono::{Local, TimeZone, Utc};
use postio_gtk::list::Row;
use postio_gtk::row::accessible_label;
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::clock;

pub fn a_rows_timestamp_follows_a_frozen_clock() {
    let now = Utc
        .with_ymd_and_hms(2026, 6, 2, 9, 0, 0)
        .unwrap()
        .with_timezone(&Local);
    let received = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    let row = Row {
        id: MessageId::new(1),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Example"), "ada@example.com")),
        subject: Some("Frozen".into()),
        preview: None,
        received_at: received,
        seen: true,
        flagged: false,
        answered: false,
        send_state: None,
        send_at: None,
        has_attachments: false,
        thread_count: 1,
        participants: Vec::new(),
    };

    let expected = postio_ui::row::timestamp(received, now);
    assert_eq!(expected, "Mon", "the pair is a day apart");

    clock::freeze(now);
    let label = accessible_label(&row);
    clock::thaw();

    assert!(
        label.ends_with(&format!("Frozen, {expected}")),
        "the row read the real clock, not the frozen one: {label:?}"
    );
}
