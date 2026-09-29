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

/// Five one-message conversations with bodies, the cursor on none yet.
async fn five_with_bodies() -> (Fixture, postio_focus::window::FocusWindow) {
    let fixture = Fixture::empty().await;
    for (n, minutes) in [10, 20, 30, 40, 50].into_iter().enumerate() {
        let subject = format!("Subject {}", n + 1);
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                &subject,
                "A line.",
                minutes,
            )
            .await;
        fixture
            .write_body(message, &format!("Body {}", n + 1))
            .await;
    }
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 5).await,
        "the inbox never reached the screen"
    );
    (fixture, window)
}

fn cursor_id(window: &postio_focus::window::FocusWindow) -> Option<postio_model::MessageId> {
    window.cursor_row().map(|row| row.id())
}

/// US2 scenario 1: open and close, and the list is as it was left -- the
/// cursor on its row, the two selected rows still selected.
pub fn escape_closes_and_keeps_the_cursor_and_the_selection() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["j", "x", "j", "x", "j"]);
        let cursor = cursor_id(&window);
        let selection = window.selection();
        enter(&window);
        assert!(window.reading().is_some_and(|reading| reading.is_open()));
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !window.reading().is_some_and(|r| r.is_open())).await,
            "Escape did not close the dialog"
        );
        assert_eq!(cursor_id(&window), cursor, "the cursor moved");
        assert_eq!(window.selection(), selection, "the selection changed");
    });
}

/// US2 scenario 2: `j` in the dialog shows the next message without
/// closing, and the list's cursor behind it moves with it; `k` comes back.
pub fn j_and_k_step_the_list_behind_the_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["j", "j", "j"]);
        enter(&window);
        let reading = window.reading().expect("open");
        assert_eq!(reading.title(), "Subject 3");
        assert_eq!(reading.subtitle(), "Message 3 of 5");

        support::keys(&window, &["j"]);
        assert!(reading.is_open(), "j closed the dialog");
        assert_eq!(reading.title(), "Subject 4");
        assert_eq!(reading.subtitle(), "Message 4 of 5");
        assert_eq!(
            window.cursor_row().and_then(|row| row
                .as_conversation()
                .and_then(|row| row.summary.subject.clone())),
            Some("Subject 4".to_owned()),
            "the list's cursor moved with the dialog"
        );
        assert!(
            crate::settle_until(async || reading.body_text().contains("Body 4")).await,
            "the next message's body never arrived: {:?}",
            reading.body_text()
        );
        support::keys(&window, &["k"]);
        assert_eq!(reading.title(), "Subject 3");
    });
}

/// US2 scenario 3: a thread opens on its latest message; `[` shows the one
/// before it and the position line says so, and `]` steps back.
pub fn brackets_step_through_the_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor API draft", 3, 5).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the thread never reached the screen"
        );
        support::keys(&window, &["j"]);
        enter(&window);
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 3")).await,
            "the thread did not open on its latest message: {:?}",
            reading.body_text()
        );
        assert_eq!(reading.subtitle(), "Message 1 of 1 \u{b7} thread of 3");
        assert!(
            crate::settle_until(async || reading.thread_known() == 3).await,
            "the conversation's messages were never read"
        );

        support::press(&window, "bracketleft", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 2")).await,
            "[ did not show the earlier message: {:?}",
            reading.body_text()
        );
        assert_eq!(
            reading.subtitle(),
            "Message 1 of 1 \u{b7} 2 of 3 in the thread"
        );
        support::press(&window, "bracketright", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 3")).await,
            "] did not step back"
        );
        assert_eq!(reading.subtitle(), "Message 1 of 1 \u{b7} thread of 3");
    });
}

/// Three one-message conversations whose bodies are long enough to scroll,
/// the cursor on the first, its message open in the dialog.
async fn three_long_open() -> (Fixture, postio_focus::window::FocusWindow) {
    let fixture = Fixture::empty().await;
    for (n, minutes) in [10, 20, 30].into_iter().enumerate() {
        let subject = format!("Long {}", n + 1);
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                &subject,
                "A line.",
                minutes,
            )
            .await;
        let body = format!("Body {}. A line long enough to scroll. ", n + 1).repeat(600);
        fixture.write_body(message, &body).await;
    }
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 3).await,
        "the inbox never reached the screen"
    );
    support::keys(&window, &["j"]);
    enter(&window);
    let reading = window.reading().expect("Enter opened the message");
    drawn_tall(&reading).await;
    (fixture, window)
}

/// Wait until the open message is drawn tall enough to scroll through.
async fn drawn_tall(reading: &std::rc::Rc<postio_focus::open::OpenMessage>) {
    assert!(
        crate::settle_until(async || {
            let view = reading.reader().view().clone();
            view.height() > 0
                && view
                    .document()
                    .is_some_and(|d| d.size.height > 3.0 * f64::from(view.height()))
        })
        .await,
        "the message was never drawn tall enough to scroll"
    );
}

fn scrolled(window: &postio_focus::window::FocusWindow) -> f64 {
    window.reading().expect("open").reader().scrolled_for_test()
}

/// T180: a message opens at its top -- the first time, and every time the
/// dialog steps to another one, however far the last was read.
pub fn a_message_opens_at_its_top_every_time() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_long_open().await;
        assert_eq!(scrolled(&window), 0.0, "the first message opens at its top");

        support::press(&window, "Page_Down", gdk::ModifierType::empty());
        support::press(&window, "Page_Down", gdk::ModifierType::empty());
        assert!(scrolled(&window) > 0.0, "the message was never read down");

        support::keys(&window, &["j"]);
        let reading = window.reading().expect("open");
        assert_eq!(reading.title(), "Long 2");
        assert!(
            crate::settle_until(async || reading.body_text().contains("Body 2")).await,
            "the next message never arrived"
        );
        drawn_tall(&reading).await;
        assert_eq!(scrolled(&window), 0.0, "the next message opens at its top");

        support::press(&window, "End", gdk::ModifierType::empty());
        // Closed at the bottom and opened again: the top, not the place
        // it was left.
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "Escape did not close the dialog"
        );
        let loads = reading.reader().loads();
        enter(&window);
        assert!(
            crate::settle_until(async || reading.reader().loads() >= loads + 2).await,
            "the reopened message was never drawn"
        );
        assert!(
            crate::settle_until(async || scrolled(&window) == 0.0).await,
            "a reopened message keeps its place: {}",
            scrolled(&window)
        );
        support::press(&window, "End", gdk::ModifierType::empty());
        support::keys(&window, &["k"]);
        assert!(
            crate::settle_until(async || reading.body_text().contains("Body 1")).await,
            "the previous message never arrived"
        );
        drawn_tall(&reading).await;
        assert_eq!(
            scrolled(&window),
            0.0,
            "the previous message opens at its top"
        );
    });
}

/// T181: while the dialog is open, the arrows and the paging keys scroll
/// its message and leave the list behind it alone; `j`/`k` still step.
pub fn arrows_and_paging_keys_scroll_the_message_not_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_long_open().await;
        let cursor = cursor_id(&window);
        let reading = window.reading().expect("open");
        let none = gdk::ModifierType::empty();

        support::press(&window, "Down", none);
        let line = scrolled(&window);
        assert!(line > 0.0, "Down did not scroll the message");
        assert_eq!(cursor_id(&window), cursor, "Down moved the list");
        assert_eq!(reading.title(), "Long 1", "Down stepped to another message");

        support::press(&window, "Page_Down", none);
        let page = scrolled(&window);
        assert!(page > line, "Page_Down did not scroll further");
        support::press(&window, "Up", none);
        assert!(scrolled(&window) < page, "Up did not scroll back");
        assert_eq!(cursor_id(&window), cursor, "Up moved the list");
        support::press(&window, "Page_Up", none);
        support::press(&window, "End", none);
        let end = scrolled(&window);
        assert!(end > page, "End did not scroll to the bottom");
        support::press(&window, "Home", none);
        assert_eq!(scrolled(&window), 0.0, "Home did not scroll to the top");
        assert_eq!(cursor_id(&window), cursor, "the list moved");
        assert_eq!(reading.title(), "Long 1");
    });
}

/// T176: `j` and `k` step to the next and previous message whichever
/// control inside the dialog has the keyboard -- the way a person's focus
/// is, after opening it, and not only when nothing is focused.
pub fn j_and_k_step_with_the_keyboard_on_each_control_of_the_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["j", "j", "j"]);
        enter(&window);
        let reading = window.reading().expect("open");
        let mut title = "Subject 3".to_owned();
        let mut visited = Vec::new();
        for _ in 0..16 {
            let dialog = reading.dialog();
            gtk::prelude::WidgetExt::child_focus(&dialog, gtk::DirectionType::TabForward);
            crate::settle();
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            visited.push(focus.as_ref().map(|widget| widget.type_().name()));
            for (key, expected) in [("j", "Subject 4"), ("k", "Subject 3")] {
                support::keys(&window, &[key]);
                assert_eq!(
                    reading.title(),
                    expected,
                    "{key} did nothing with the keyboard on {visited:?}"
                );
            }
            title = reading.title();
        }
        assert_eq!(title, "Subject 3");
        assert!(visited.len() > 3, "the keyboard never moved: {visited:?}");
    });
}

/// T179: the toolbar's Delete deletes the message on screen, and so does its
/// key, in the dialog as over the list.
pub fn delete_in_the_dialog_deletes_the_message_on_screen() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_with_bodies().await;
        enter(&window);
        let reading = window.reading().expect("open");
        let delete = support::only(&reading.dialog(), "focus-open-delete")
            .downcast::<gtk::Button>()
            .expect("a button");
        delete.emit_clicked();
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Harbor draft", "Staffing"])
                .await,
            "the toolbar's Delete left the message in the inbox: {:?}",
            support::subjects(&window)
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "Escape did not close the dialog"
        );
        support::keys(&window, &["j"]);
        enter(&window);
        support::press(&window, "Delete", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the Delete key did nothing in the dialog: {:?}",
            support::subjects(&window)
        );
    });
}
