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

/// T136, US10 scenario 3: a delivered digest is one row among the
/// conversations, where it came due -- below the conversations newer than
/// that -- saying its cadence, its rule, its count and its senders. What
/// it holds is not listed besides.
pub fn a_delivered_digest_is_one_row_where_it_came_due() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for (from, subject, minutes) in [
            (("Ada Moreno", "ada@example.com"), "Atlas budget", 30),
            (("Lena Park", "lena@example.org"), "Harbor draft", 20),
            (("Tomás Reyes", "tomas@example.net"), "Staffing plan", 10),
        ] {
            fixture.file(from, subject, "Hello.", minutes).await;
        }
        let mut held = Vec::new();
        for (subject, minutes) in [("The weekly numbers", 40), ("The rate decision", 50)] {
            let (message, _) = fixture
                .file(("Ledger", "news@ledger.test"), subject, "Rates.", minutes)
                .await;
            held.push(message);
        }
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let digests = postio_storage::repository::DigestRepository::new(&connection);
            for message in &held {
                digests
                    .hold(*message, "Newsletters", support::now())
                    .await
                    .expect("held");
            }
            digests
                .deliver(
                    "Newsletters",
                    support::now() - Duration::minutes(15),
                    support::now() - Duration::minutes(15),
                )
                .await
                .expect("delivered")
                .expect("a delivery");
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || {
                support::subjects(&window)
                    == [
                        "Staffing plan",
                        "digest: Newsletters",
                        "Harbor draft",
                        "Atlas budget",
                    ]
            })
            .await,
            "the digest is not one row where it came due: {:?}",
            support::subjects(&window)
        );
        let pane = window.pane().expect("the list");
        let digest_said = || {
            pane.rows_on_screen()
                .get(1)
                .map(|row| row.drawn().texts)
                .unwrap_or_default()
        };
        assert!(
            crate::settle_until(async || !digest_said().is_empty()).await,
            "the digest row never drew"
        );
        let said = digest_said();
        for wanted in ["Digest", "Newsletters \u{b7} 2 messages", "From Ledger"] {
            assert!(
                said.iter().any(|text| text.starts_with(wanted)),
                "the digest row does not say {wanted:?}: {said:?}"
            );
        }

        // `a` on the digest's row archives the whole delivery, and the row
        // goes; what it held does not come back to the inbox.
        support::keys(&window, &["j"]);
        support::press(&window, "a", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Staffing plan", "Harbor draft", "Atlas budget"]
            })
            .await,
            "archiving the digest left: {:?}",
            support::subjects(&window)
        );
    });
}
