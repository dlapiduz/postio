//! Surfaced rows (US5 scenario 4, T095): a reminder that found no reply by
//! its time comes back to the top of the inbox as a row of its own, marked
//! "No reply since …", at its place among the conversations -- and the
//! conversation is not listed a second time.

use chrono::Duration;
use postio_storage::repository::ReminderRepository;

use crate::support::{self, Fixture};

pub fn a_fired_reminder_is_a_no_reply_row_at_its_place_and_listed_once() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (budget, thread) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "The numbers are attached.",
                30,
            )
            .await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Uploaded the second draft.",
                20,
            )
            .await;
        fixture
            .file(
                ("Tomás Reyes", "tomas@example.net"),
                "Staffing plan",
                "Sharing the draft before Monday.",
                10,
            )
            .await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let reminders = ReminderRepository::new(&connection);
            let set_at = support::now() - Duration::days(3);
            let id = reminders
                .set(
                    thread,
                    budget,
                    support::now() - Duration::minutes(5),
                    set_at,
                )
                .await
                .expect("set");
            reminders
                .fire(id, support::now() - Duration::minutes(5))
                .await
                .expect("fired");
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Atlas budget", "Staffing plan", "Harbor draft"]
            })
            .await,
            "the reminder's row is on top, and the conversation once: {:?}",
            support::subjects(&window)
        );
        let pane = window.pane().expect("the list");
        let top_said = || {
            pane.rows_on_screen()
                .into_iter()
                .next()
                .map(|top| top.drawn().texts)
                .unwrap_or_default()
        };
        assert!(
            crate::settle_until(async || !top_said().is_empty()).await,
            "the top row never drew"
        );
        let said = top_said();
        assert!(
            said.iter().any(|text| text == "No reply"),
            "the row says no one replied: {said:?}"
        );
        assert!(
            said.iter().any(|text| text.starts_with("since ")),
            "and since when: {said:?}"
        );

        // Archived from its row, the reminder goes with its conversation.
        support::press(&window, "j", gtk::gdk::ModifierType::empty());
        assert_eq!(
            window.cursor_row().map(|row| row.id()),
            Some(budget),
            "the cursor is on the reminder's row"
        );
        support::press(&window, "a", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Staffing plan", "Harbor draft"]
            })
            .await,
            "archived, the reminder's row left: {:?}",
            support::subjects(&window)
        );
    });
}
