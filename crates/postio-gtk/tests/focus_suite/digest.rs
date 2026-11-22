use chrono::Duration;
use gtk::gdk;

use crate::support::{self, Fixture};

async fn delivered() -> Fixture {
    delivered_holding().await.0
}

pub async fn delivered_holding() -> (
    Fixture,
    postio_model::DeliveryId,
    Vec<postio_model::MessageId>,
) {
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
    let connection = fixture.database.connect().await.expect("a connection");
    let digests = postio_storage::repository::DigestRepository::new(&connection);
    for message in &held {
        digests
            .hold(*message, "Newsletters", support::now())
            .await
            .expect("held");
    }
    let delivery = digests
        .deliver(
            "Newsletters",
            support::now() - Duration::minutes(15),
            support::now() - Duration::minutes(15),
        )
        .await
        .expect("delivered")
        .expect("a delivery");
    drop(connection);
    (fixture, delivery, held)
}

pub async fn open_digest(
    window: &postio_gtk::window::FocusWindow,
) -> std::rc::Rc<postio_gtk::digest::DigestWindow> {
    assert!(
        crate::settle_until(async || support::subjects(window).len() == 4).await,
        "the inbox and its digest never reached the screen: {:?}",
        support::subjects(window)
    );
    support::keys(window, &["j"]);
    support::press(window, "Return", gtk::gdk::ModifierType::empty());
    let digest = window.digest().expect("Enter opened the digest");
    assert!(
        crate::settle_until(async || digest.subjects().len() == 2).await,
        "the digest never listed what it holds: {:?}",
        digest.subjects()
    );
    digest
}

pub fn enter_opens_a_digest_and_e_archives_all_of_it() {
    crate::gtk_case(async {
        let fixture = delivered().await;
        let (window, _client) = fixture.open().await;
        let digest = open_digest(&window).await;
        assert_eq!(
            digest.subjects(),
            ["The weekly numbers", "The rate decision"],
            "newest first"
        );
        assert_eq!(
            digest.showing(),
            postio_gtk::digest::DigestPage::List,
            "no summary, the list"
        );
        assert!(
            digest.texts().iter().all(|text| text != "Summary"),
            "a Summary tab with no summary: {:?}",
            digest.texts()
        );
        let first = digest.focused().map(|row| row.id);
        support::press(&window, "j", gdk::ModifierType::empty());
        assert!(
            digest.focused().is_some() && digest.focused().map(|row| row.id) != first,
            "j did not move in the digest"
        );
        let said = digest.texts();
        for wanted in ["Newsletters", "Archive all 2", "2 messages from 1 sender"] {
            assert!(
                said.iter().any(|text| text.contains(wanted)),
                "no {wanted:?} in {said:?}"
            );
        }
        assert!(
            support::keycaps_are_taught(digest.dialog()),
            "the digest window's keycaps do not teach their controls (T142)"
        );
        support::press(&window, "e", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Staffing plan", "Harbor draft", "Atlas budget"]
            })
            .await,
            "the digest's row stayed: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || window.digest().is_none()).await,
            "archiving all of it closed the window"
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "one Ctrl+Z did not bring the digest back: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || {
                matches!(
                    window.cursor_row(),
                    Some(postio_gtk::list::FocusRow::Digest(_))
                )
            })
            .await,
            "the cursor is not on the digest's row, which came back: {:?}",
            window.cursor_row().map(|row| row.id())
        );
    });
}
