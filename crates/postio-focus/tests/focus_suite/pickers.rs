//! The pickers at the row (US5; screens 11-14): `s`, `h`, `l` and `m` open
//! a small picker anchored to the focused row, acting on the selection when
//! there is one.

use chrono::Local;
use gtk::gdk;
use postio_storage::repository::MessageRepository;

use crate::support::{self};

/// US5 scenario 1, in Focus: `s` opens the snooze picker at the row, naming
/// the conversation; `2` snoozes it to tomorrow morning, and it leaves the
/// inbox.
pub fn s_then_2_snoozes_the_row_until_tomorrow_morning() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let subject = support::subjects(&window)[0].clone();
        support::press(&window, "s", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("s opened the snooze picker");
        assert!(
            crate::settle_until(async || picker.is_shown()).await,
            "the snooze picker never drew"
        );
        let said = picker.texts();
        assert!(said.iter().any(|line| line == "Snooze until"), "{said:?}");
        assert!(
            said.iter().any(|line| line.ends_with(&subject)),
            "the picker names what it snoozes: {said:?}"
        );
        let expected = postio_ui::schedule::snooze_presets(Local::now())[1].1;
        support::press(&window, "2", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !support::subjects(&window).contains(&subject)).await,
            "the snoozed conversation stayed in the inbox"
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let stored = MessageRepository::new(&connection)
            .get(row.id())
            .await
            .expect("a read")
            .expect("the message");
        assert_eq!(
            stored.snoozed_until,
            Some(expected.to_utc()),
            "snoozed until tomorrow morning"
        );
    });
}

/// `h` opens the remind picker at the row; `3` sets a reminder for the end
/// of the week on the conversation, which stays in the inbox until then.
pub fn h_then_3_reminds_at_the_end_of_the_week() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "j", gdk::ModifierType::empty());
        let row = window.cursor_row().expect("j put the cursor on a row");
        let thread = row.thread().expect("a conversation");
        support::press(&window, "h", gdk::ModifierType::empty());
        let picker = window.open_picker().expect("h opened the remind picker");
        assert!(crate::settle_until(async || picker.is_shown()).await);
        let said = picker.texts();
        assert!(
            said.iter()
                .any(|line| line == "Remind me if no one replies by"),
            "{said:?}"
        );
        let expected = postio_ui::schedule::remind_presets(Local::now())[2].1;
        support::press(&window, "3", gdk::ModifierType::empty());
        let connection = fixture.database.connect().await.expect("a connection");
        let reminders = postio_storage::repository::ReminderRepository::new(&connection);
        let mut standing = None;
        for _ in 0..200 {
            standing = reminders.standing(thread).await.expect("a read");
            if standing.is_some() {
                break;
            }
            crate::settle();
        }
        let standing = standing.expect("no reminder was set");
        assert_eq!(
            standing.due_at,
            expected.to_utc(),
            "due at the end of the week"
        );
        assert!(!picker.is_open(), "choosing closed the picker");
    });
}
