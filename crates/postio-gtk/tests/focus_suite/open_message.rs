use adw::prelude::*;
use gtk::gdk;

use crate::support::{self, Fixture};

async fn three_with_bodies() -> (Fixture, postio_gtk::window::FocusWindow) {
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
    (fixture, window)
}

fn enter(window: &postio_gtk::window::FocusWindow) {
    support::press(window, "Return", gtk::gdk::ModifierType::empty());
}

pub fn enter_opens_the_conversation_over_the_list_at_once() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_with_bodies().await;
        enter(&window);
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
        for key in ["r", "a", "f", "e", "b", "h", "l", "v"] {
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

pub fn a_hundredth_open_of_a_thread_builds_no_second_message_view() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 4, 10).await;
        for id in &ids {
            fixture.write_body(*id, "A line.").await;
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let before = postio_ui::test_support::surfaces_created();
        for _ in 0..100 {
            enter(&window);
            assert!(
                crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
                "Enter did not open the thread"
            );
            support::press(&window, "Escape", gdk::ModifierType::empty());
            assert!(
                crate::settle_until(async || !window.reading().is_some_and(|r| r.is_open())).await,
                "Escape did not close the dialog"
            );
        }
        assert_eq!(
            postio_ui::test_support::surfaces_created() - before,
            1,
            "a hundred opens of a thread built more than the one message view"
        );
    });
}

async fn five_with_bodies() -> (Fixture, postio_gtk::window::FocusWindow) {
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

fn cursor_id(window: &postio_gtk::window::FocusWindow) -> Option<postio_model::MessageId> {
    window.cursor_row().map(|row| row.id())
}

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

pub fn j_and_k_step_the_list_behind_the_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["j", "j"]);
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

pub fn brackets_step_through_the_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let thread = fixture.thread_of("Harbor API draft", 3, 5).await;
        fixture.read_all_but_last(&thread).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the thread never reached the screen"
        );
        support::keys(&window, &["j"]);
        enter(&window);
        assert!(
            crate::settle_until(async || window.reading().is_some()).await,
            "the thread never opened"
        );
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 3")).await,
            "the thread did not open on its latest message: {:?}",
            reading.body_text()
        );
        assert!(
            crate::settle_until(async || reading.subtitle() == "Message 3 of 3").await,
            "the title line: {:?}",
            reading.subtitle()
        );
        assert!(
            crate::settle_until(async || reading.thread_known() == 3).await,
            "the conversation's messages were never read"
        );

        support::press(&window, "p", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 2")).await,
            "p did not show the earlier message: {:?}",
            reading.body_text()
        );
        assert!(
            crate::settle_until(async || reading.subtitle() == "Message 2 of 3").await,
            "the title line after p: {:?}",
            reading.subtitle()
        );
        assert_eq!(
            reading.thread_chip_said(),
            "Message 2 of 3 \u{b7} ] newer \u{b7} [ older",
            "the line above the subject says where the dialog is"
        );
        support::press(&window, "n", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 3")).await,
            "n did not step back"
        );
        assert!(
            crate::settle_until(async || reading.subtitle() == "Message 3 of 3").await,
            "the title line after n: {:?}",
            reading.subtitle()
        );
        assert_eq!(reading.thread_chip_said(), "Message 3 of 3 \u{b7} [ older");
    });
}

async fn three_long_open() -> (Fixture, postio_gtk::window::FocusWindow) {
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
    enter(&window);
    let reading = window.reading().expect("Enter opened the message");
    drawn_tall(&reading).await;
    (fixture, window)
}

async fn drawn_tall(reading: &std::rc::Rc<postio_gtk::open::OpenMessage>) {
    assert!(
        crate::settle_until(async || {
            let view = reading.reader().view().clone();
            let Some(column) = view.ancestor(gtk::ScrolledWindow::static_type()) else {
                return false;
            };
            let Some(column) = column.downcast_ref::<gtk::ScrolledWindow>() else {
                return false;
            };
            let scroll = column.vadjustment();
            scroll.page_size() > 0.0
                && scroll.upper() > 3.0 * scroll.page_size()
                && view
                    .document()
                    .is_some_and(|d| d.size.height > 3.0 * scroll.page_size())
        })
        .await,
        "the message was never drawn tall enough to scroll"
    );
}

fn scrolled(window: &postio_gtk::window::FocusWindow) -> f64 {
    window.reading().expect("open").reader().scrolled_for_test()
}

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

pub fn j_and_k_step_with_the_keyboard_on_each_control_of_the_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["j", "j"]);
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
        support::click(&window, &delete, 1);
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

pub fn deleting_from_the_dialog_moves_to_the_next_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = three_with_bodies().await;
        enter(&window);
        let reading = window.reading().expect("open");
        assert_eq!(reading.title(), "Budget");

        support::press(&window, "Delete", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || reading.title() == "Harbor draft").await,
            "the deleted message is still on screen: {:?}",
            reading.title()
        );
        assert_eq!(support::subjects(&window), ["Harbor draft", "Staffing"]);
        assert!(reading.is_open());
        assert!(
            crate::settle_until(async || reading.body_text().contains("pagination")).await,
            "the next message's body never arrived: {:?}",
            reading.body_text()
        );

        support::keys(&window, &["j"]);
        assert!(crate::settle_until(async || reading.title() == "Staffing").await);
        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || reading.title() == "Harbor draft").await,
            "at the end the dialog should show the previous: {:?}",
            reading.title()
        );
        assert_eq!(support::subjects(&window), ["Harbor draft"]);

        support::press(&window, "Delete", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || !reading.is_open()).await,
            "with nothing left the dialog should close"
        );
        assert!(support::subjects(&window).is_empty());
    });
}

pub fn a_draft_from_another_client_offers_no_reply() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Unsent thoughts",
                "Half a sentence.",
                10,
            )
            .await;
        fixture.write_body(message, "Half a sentence.").await;
        let connection = fixture.database.connect().await.expect("a connection");
        let mut flags = postio_model::FlagSet::new();
        flags.insert(postio_model::Flag::Draft);
        postio_storage::repository::MessageRepository::new(&connection)
            .set_flags(
                message,
                &flags,
                postio_storage::repository::FlagSource::Server,
            )
            .await
            .expect("the draft flag");
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        enter(&window);
        let reading = window.reading().expect("Enter opened the message");
        assert!(
            crate::settle_until(async || reading.foreign_draft()).await,
            "the message never showed as another client's draft"
        );
        let labelled = |label: &str| {
            support::descendants(&reading.dialog())
                .into_iter()
                .any(|widget| {
                    widget.is::<gtk::Button>()
                        && widget.is_mapped()
                        && support::texts(&widget).iter().any(|text| text == label)
                })
        };
        for kept in ["Archive", "Delete"] {
            assert!(
                crate::settle_until(async || labelled(kept)).await,
                "no {kept:?} on a draft from another client: {:?}",
                support::texts(&reading.dialog())
            );
        }
        for gone in ["Reply", "Reply all", "Forward"] {
            assert!(
                !labelled(gone),
                "{gone:?} is offered on a draft from another client"
            );
        }
        support::deliver(&window, "r");
        crate::settle();
        assert!(
            window.compose_dialog().is_none(),
            "r opened a composer on a draft from another client"
        );
    });
}

/// A verb in the open message is for that message, never the rows selected behind it (the
/// controller's rule, as on the Mac).
pub fn archive_in_the_dialog_archives_the_message_not_the_selection() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = five_with_bodies().await;
        support::keys(&window, &["x", "j", "j"]);
        let selected = window.selection();
        assert_eq!(
            support::subjects(&window).len(),
            5,
            "five rows before the archive"
        );
        enter(&window);
        let reading = window.reading().expect("open");
        assert_eq!(reading.title(), "Subject 3");

        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "the archive never reached the list: {:?}",
            support::subjects(&window)
        );
        assert_eq!(
            support::subjects(&window),
            ["Subject 1", "Subject 2", "Subject 4", "Subject 5"],
            "the message on screen went, not the selected row"
        );
        assert!(
            crate::settle_until(async || reading.title() == "Subject 4").await,
            "the dialog should go on to the next: {:?}",
            reading.title()
        );
        assert_eq!(window.selection(), selected, "the selection was spent");
    });
}
