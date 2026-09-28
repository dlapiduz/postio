//! Opening a message over the list (US2, T069; screen 04): `Enter` opens
//! the conversation under the cursor in a dialog at once -- its header from
//! the row already in hand, the body from the store when it lands -- and a
//! hundred opens reuse the one message view (scenario 7).

use adw::prelude::*;
use gtk::gdk;

use crate::support::{self, Fixture};

/// A window over three conversations with bodies, the cursor on the first.
async fn three_with_bodies() -> (Fixture, postio_focus::window::FocusWindow) {
    let fixture = Fixture::empty().await;
    for (subject, body, minutes) in [
        (
            "Budget",
            "The numbers are attached. Can you approve them by Friday?",
            10,
        ),
        (
            "Harbor draft",
            "Uploaded v3 with the pagination changes.",
            20,
        ),
        ("Staffing", "Sharing the plan before Monday.", 30),
    ] {
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), subject, body, minutes)
            .await;
        fixture.write_body(message, body).await;
    }
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 3).await,
        "the inbox never reached the screen"
    );
    support::keys(&window, &["j"]);
    (fixture, window)
}

fn enter(window: &postio_focus::window::FocusWindow) {
    let _ = window.handle_key(gdk::Key::Return, gdk::ModifierType::empty());
}

pub fn enter_opens_the_conversation_over_the_list_at_once() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_with_bodies().await;
        enter(&window);
        // No turn of the main loop: the header is the row's, in hand.
        let reading = window.reading().expect("Enter opened the message");
        assert!(reading.is_open(), "the dialog is up");
        assert_eq!(reading.title(), "Budget");
        assert_eq!(reading.subtitle(), "Message 1 of 3");

        assert!(
            crate::settle_until(async || reading.body_text().contains("approve them by Friday"))
                .await,
            "the body never arrived: {:?}",
            reading.body_text()
        );
        // The sheet maps its content once it has opened.
        assert!(
            crate::settle_until(async || {
                reading
                    .dialog()
                    .child()
                    .is_some_and(|content| gtk::prelude::WidgetExt::is_mapped(&content))
            })
            .await,
            "the dialog's content was never shown"
        );
        let said = support::texts(&reading.dialog()).join(" | ");
        for expected in [
            "Ada Moreno",
            "ada@example.com",
            "Reply",
            "Reply all",
            "Forward",
            "Archive",
        ] {
            assert!(said.contains(expected), "no {expected:?} in {said}");
        }
        for key in ["e", "E", "f", "a", "s", "h", "l", "m"] {
            assert!(
                support::texts(&reading.dialog()).contains(&key.to_owned()),
                "the toolbar shows no key {key}: {said}"
            );
        }
    });
}

pub fn a_hundredth_open_builds_no_second_message_view() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_with_bodies().await;
        let before = postio_ui::test_support::surfaces_created();
        for _ in 0..100 {
            enter(&window);
            assert!(window.reading().is_some_and(|reading| reading.is_open()));
            support::press(&window, "Escape", gdk::ModifierType::empty());
            assert!(
                crate::settle_until(async || !window.reading().is_some_and(|r| r.is_open())).await,
                "Escape did not close the dialog"
            );
        }
        assert_eq!(
            postio_ui::test_support::surfaces_created() - before,
            1,
            "a hundred opens built more than the one message view"
        );
    });
}
