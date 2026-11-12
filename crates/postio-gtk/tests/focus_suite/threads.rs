//! A thread opens in place (specs/013-focus-threads US1; issue #2099): `Right`
//! splices its messages in under its row, `j` walks into them, `Left` folds
//! them, and `Return` on one opens that message. Real keys in the real
//! window; what is asserted is what a person sees -- the rows drawn and the
//! observation's account of where the cursor is.

use crate::support::{self, Fixture};
use gtk::prelude::*;
use postio_model::MessageId;

/// "Harbor plan", a thread of four, above "Solo", a single message.
async fn harbor() -> (
    Fixture,
    postio_gtk::window::FocusWindow,
    Vec<MessageId>,
    postio_gtk::list::ListPane,
) {
    let fixture = Fixture::empty().await;
    let ids = fixture.thread_of("Harbor plan", 4, 10).await;
    fixture.file_in(fixture.inbox, "Solo", 300).await;
    let (window, _client) = fixture.open().await;
    let pane = window.pane().expect("the inbox");
    assert!(
        crate::settle_until(async || pane.cursor().selected() == 0 && pane.cursor().n_items() == 2)
            .await,
        "the inbox opens with the thread above the single message"
    );
    (fixture, window, ids, pane)
}

fn said(window: &postio_gtk::window::FocusWindow, key: &str) -> Option<serde_json::Value> {
    window.observe().app.get(key).cloned()
}

/// Every text the rows on screen drew, flattened.
fn drawn(pane: &postio_gtk::list::ListPane) -> Vec<String> {
    pane.rows_on_screen()
        .iter()
        .flat_map(|row| row.drawn().texts)
        .collect()
}

pub fn right_opens_a_thread_in_place() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        support::keys(&window, &["Right"]);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 6).await,
            "Right spliced the four messages in under the thread: {} rows",
            pane.cursor().n_items()
        );
        assert!(
            window.reading().is_none(),
            "Right expands; it does not open the message"
        );
        assert_eq!(pane.cursor().selected(), 0, "the cursor stays on the row");
        assert_eq!(
            said(&window, "focus.cursor.kind"),
            Some(serde_json::json!("row"))
        );
        assert!(
            crate::settle_until(async || {
                let texts = drawn(&pane);
                (1..=4).all(|n| texts.contains(&format!("Message {n}")))
            })
            .await,
            "each child shows its own line: {:?}",
            drawn(&pane)
        );
    });
}

pub fn j_walks_the_children_and_on_to_the_next_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::keys(&window, &["j"]);
        assert_eq!(pane.cursor().selected(), 1, "j goes into the children");
        assert_eq!(
            said(&window, "focus.cursor.kind"),
            Some(serde_json::json!("child"))
        );
        assert_eq!(
            said(&window, "focus.cursor.line"),
            Some(serde_json::json!("Message 1"))
        );
        support::keys(&window, &["j", "j", "j"]);
        assert_eq!(pane.cursor().selected(), 4, "one child at a time");
        assert_eq!(
            said(&window, "focus.cursor.line"),
            Some(serde_json::json!("Message 4"))
        );
        support::keys(&window, &["j"]);
        assert_eq!(pane.cursor().selected(), 5, "then on to the next thread");
        assert_eq!(
            said(&window, "focus.cursor.kind"),
            Some(serde_json::json!("row"))
        );
        support::keys(&window, &["k"]);
        assert_eq!(pane.cursor().selected(), 4, "k walks back into them");
    });
}

pub fn left_folds_the_thread_from_a_child() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::keys(&window, &["j", "j"]);
        assert_eq!(pane.cursor().selected(), 2);
        support::keys(&window, &["Left"]);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 2).await,
            "Left folded the children away"
        );
        assert_eq!(pane.cursor().selected(), 0, "the cursor is back on the row");
        assert_eq!(
            said(&window, "focus.cursor.kind"),
            Some(serde_json::json!("row"))
        );
        assert!(
            !drawn(&pane).contains(&"Message 2".to_owned()),
            "no child is left on screen"
        );
    });
}

pub fn return_on_a_child_opens_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, ids, pane) = harbor().await;
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::keys(&window, &["j", "j"]);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || window.observe().reading.id.is_some()).await,
            "Return opened a message"
        );
        assert_eq!(
            window.observe().reading.id,
            Some(ids[1].get().to_string()),
            "the second message, not the latest"
        );
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.observe().reading.id.is_none()).await,
            "Escape closed it"
        );
        assert_eq!(
            pane.cursor().selected(),
            2,
            "the ring is on the child again"
        );
        assert_eq!(pane.cursor().n_items(), 6, "and the thread is still open");
    });
}

/// The first row's drawn words, once it is drawn.
async fn first_row(pane: &postio_gtk::list::ListPane) -> Vec<String> {
    assert!(
        crate::settle_until(async || {
            pane.rows_on_screen()
                .first()
                .is_some_and(|row| !row.drawn().texts.is_empty())
        })
        .await,
        "the first row was never drawn"
    );
    pane.rows_on_screen()[0].drawn().texts
}

pub fn participants_say_you_for_the_account() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_with_you("Plans", 10).await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || first_row(&pane).await[0] == "Ada, you").await,
            "the participants never said \"you\": {:?}",
            first_row(&pane).await
        );
    });
}

pub fn the_count_follows_the_participants() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, _window, _ids, pane) = harbor().await;
        let texts = first_row(&pane).await;
        assert_eq!(
            texts.get(1).map(String::as_str),
            Some("4"),
            "the badge is drawn right after the participants, before the subject: {texts:?}"
        );
    });
}

pub fn clicking_the_chevron_opens_and_folds_a_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        let click = |window: &postio_gtk::window::FocusWindow| {
            let row = pane.rows_on_screen()[0].clone();
            // The chevron sits in the gutter between the unread dot and the names.
            let x = 46.0;
            support::click_in(window, &row, x, row.height() as f32 / 2.0, 1);
        };
        assert!(
            crate::settle_until(
                async || !pane.rows_on_screen().is_empty() && pane.rows_on_screen()[0].height() > 0
            )
            .await
        );
        click(&window);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 6).await,
            "a click on the chevron opened the thread: {} rows",
            pane.cursor().n_items()
        );
        assert_eq!(pane.cursor().selected(), 0, "the cursor is on the row");
        assert!(window.reading().is_none(), "and nothing was opened");
        assert!(
            crate::settle_until(async || pane.rows_on_screen()[0].is_expanded()).await,
            "the row draws its chevron turned"
        );
        click(&window);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 2).await,
            "a second click folded it"
        );
    });
}

/// The child rows Filtered draws, top to bottom.
fn filtered_children(
    view: &postio_gtk::filtered::FilteredView,
) -> Vec<postio_gtk::list::row::RowWidget> {
    support::descendants(view.widget())
        .into_iter()
        .filter_map(|widget| widget.downcast::<postio_gtk::list::row::RowWidget>().ok())
        .filter(|row| row.child().is_some())
        .collect()
}

pub fn filtered_opens_a_thread_in_place_and_restores_it_whole() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        fixture
            .filtered_thread("Build status", 3, "notification", 10)
            .await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || window
                .filtered()
                .is_some_and(|view| view.subjects().len() == 1))
            .await,
            "Filtered never listed the thread's row"
        );
        let view = window.filtered().expect("the view");
        support::keys(&window, &["Right"]);
        assert!(
            crate::settle_until(async || filtered_children(&view).len() == 3).await,
            "Right showed the three messages under the row: {}",
            filtered_children(&view).len()
        );
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || filtered_children(&view)
                .iter()
                .any(|row| row.is_cursor_here()
                    && row.child().is_some_and(|child| child.line == "Message 1")))
            .await,
            "j did not land on the first child"
        );
        support::keys(&window, &["Left"]);
        assert!(
            crate::settle_until(async || filtered_children(&view).is_empty()).await,
            "Left did not fold the thread"
        );
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || filtered_children(&view).len() == 3).await);
        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || filtered_children(&view)
                .iter()
                .any(|row| row.is_cursor_here()))
            .await
        );
        support::press(&window, "R", gtk::gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || view.subjects().is_empty()).await,
            "Restore on a child did not take the thread out of Filtered: {:?}",
            view.subjects()
        );
    });
}

pub fn the_footer_says_what_r_acts_on_and_r_on_a_child_quotes_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        assert!(
            crate::settle_until(async || said(&window, "focus.footer.title")
                == Some(serde_json::json!("Re: Harbor plan \u{b7} 4 messages")))
            .await,
            "the footer names the thread on its row: {:?}",
            said(&window, "focus.footer.title")
        );
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::keys(&window, &["j", "j"]);
        assert_eq!(
            said(&window, "focus.footer.title"),
            Some(serde_json::json!("Message 2 of 4 \u{b7} Ada Moreno")),
            "the footer names the child the cursor is on"
        );
        assert!(
            said(&window, "focus.footer.note")
                .and_then(|note| note.as_str().map(str::to_owned))
                .is_some_and(|note| note.starts_with("Reply and forward act on this message")),
            "and says what acts on what"
        );
        support::keys(&window, &["r"]);
        assert!(
            crate::settle_until(async || window.observe().composer.open).await,
            "r on a child opened a reply"
        );
        let body = window
            .composer()
            .expect("the composer")
            .draft()
            .body
            .text
            .unwrap_or_default();
        assert!(
            body.contains("Message 2") && !body.contains("Message 4"),
            "the reply quotes the child, not the latest: {body:?}"
        );
    });
}

fn reading_said(window: &postio_gtk::window::FocusWindow, key: &str) -> Option<serde_json::Value> {
    said(window, &format!("focus.reading.{key}"))
}

pub fn the_window_walks_its_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        fixture.read_first(&ids, 4).await;
        let solo = fixture.file_in(fixture.inbox, "Solo", 600).await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(
                async || pane.cursor().selected() == 0 && pane.cursor().n_items() == 2
            )
            .await
        );
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(4))
            )
            .await,
            "the window opened on the first unread, message 5: {:?}",
            reading_said(&window, "chip")
        );
        assert_eq!(reading_said(&window, "chips"), Some(serde_json::json!(6)));
        assert_eq!(
            reading_said(&window, "kicker"),
            Some(serde_json::json!(
                "Message 5 of 6 \u{b7} ] newer \u{b7} [ older"
            ))
        );
        assert_eq!(
            reading_said(&window, "banner"),
            Some(serde_json::json!(true))
        );

        for _ in 0..2 {
            support::press(&window, "bracketleft", gtk::gdk::ModifierType::empty());
        }
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(2))
            )
            .await,
            "[ [ showed message 3: {:?}",
            reading_said(&window, "chip")
        );
        assert_eq!(
            reading_said(&window, "banner"),
            Some(serde_json::json!(true))
        );
        assert_eq!(pane.cursor().selected(), 0, "the list's cursor stayed");
        assert_eq!(
            window.reading().expect("open").subtitle(),
            "Message 3 of 6 \u{b7} ] newer \u{b7} [ older",
            "the header says where the message is in its thread, not a result"
        );

        support::press(&window, "braceright", gtk::gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(5))
            )
            .await,
            "}} showed the latest: {:?}",
            reading_said(&window, "chip")
        );
        assert_eq!(
            reading_said(&window, "banner"),
            Some(serde_json::json!(false))
        );

        support::keys(&window, &["g", "u"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(4))
            )
            .await,
            "g u showed the first unread: {:?}",
            reading_said(&window, "chip")
        );

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(
                async || window.observe().reading.id == Some(solo.get().to_string())
            )
            .await,
            "j in the window opened the next thread: {:?}",
            window.observe().reading.id
        );
        assert_eq!(
            reading_said(&window, "chips"),
            Some(serde_json::json!(0)),
            "a thread of one has no bar"
        );
    });
}
