//! A thread opens in place (specs/013-focus-threads US1; issue #2099): `Right`
//! splices its messages in under its row, `j` walks into them, `Left` folds
//! them, and `Return` on one opens that message. Real keys in the real
//! window; what is asserted is what a person sees -- the rows drawn and the
//! observation's account of where the cursor is.

use crate::support::{self, Fixture};
use adw::prelude::AdwDialogExt as _;
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
                .is_some_and(|note| note.ends_with(" act on the thread")),
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
        assert_eq!(
            window.reading().expect("open").subtitle(),
            "Message 5 of 6",
            "the header says where the message is in its thread, as the kicker does"
        );
        assert!(
            window
                .observe()
                .app
                .get("focus.footer.title")
                .and_then(|title| title.as_str())
                .is_some_and(|title| title.starts_with("Message 5 of 6")),
            "the footer behind the window names the message r acts on: {:?}",
            window.observe().app.get("focus.footer.title")
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
            "Message 3 of 6",
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
        assert_eq!(
            window.reading().expect("open").subtitle(),
            "Message 6 of 6",
            "the latest is still said by its place in the thread, not the inbox's"
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

/// Open the first row's message in a window over `fixture`, with the sink
/// its events can be told through.
async fn opened_telling(
    fixture: &Fixture,
) -> (
    postio_gtk::window::FocusWindow,
    postio_core::bridge::EventSink,
) {
    let (host, sink) = fixture.host_telling();
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    support::keep(postio_gtk::startup::adopt(
        &window,
        host,
        &postio_config::Config::default(),
    ));
    assert!(
        crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
        "the inbox never reached the screen"
    );
    (window, sink)
}

pub fn a_reply_while_reading_changes_nothing_on_screen_until_read_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 4, 10).await;
        let (window, sink) = opened_telling(&fixture).await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chips") == Some(serde_json::json!(4))
            )
            .await
        );
        support::press(&window, "bracketright", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(1))
            )
            .await
        );
        let shown = window.observe().reading.id;
        // Where the body is drawn, in the window: a reply must not move it.
        let body_y = |window: &postio_gtk::window::FocusWindow| {
            let reading = window.reading().expect("open");
            let dialog = reading.dialog().child().expect("the dialog's content");
            reading
                .reader()
                .view()
                .compute_bounds(&dialog)
                .map(|bounds| bounds.y())
        };
        // The message is whole on screen before it is measured: a body
        // that lands later is its own layout, not the reply's.
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.reader().view().document().is_some()
                && !reading.body_text().is_empty()
                && reading.reader().view().tiles_settled())
            .await,
            "the body was never drawn"
        );
        crate::settle();
        let before = body_y(&window);
        assert!(before.is_some(), "the body is on screen");

        let (reply, thread) = fixture.reply_to(*ids.last().unwrap(), "Ana replied").await;
        sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![reply],
        });
        sink.emit(postio_core::Event::ThreadChanged {
            account: fixture.account.id,
            thread,
            members_added: vec![reply],
        });
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chips") == Some(serde_json::json!(5))
            )
            .await,
            "the bar never gained the reply's chip"
        );
        assert_eq!(
            reading_said(&window, "new_reply"),
            Some(serde_json::json!(true)),
            "the banner says a reply came"
        );
        assert_eq!(
            window.observe().reading.id,
            shown,
            "the shown message is unchanged"
        );
        assert_eq!(reading_said(&window, "chip"), Some(serde_json::json!(1)));
        crate::settle();
        assert_eq!(
            body_y(&window),
            before,
            "the banner for the reply must not push the text being read down"
        );

        support::press(&window, "braceright", gtk::gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || reading_said(&window, "chip")
                == Some(serde_json::json!(4))
                && reading_said(&window, "new_reply") == Some(serde_json::json!(false)))
            .await,
            "Read it showed the reply and took the banner away"
        );
    });
}

pub fn the_thread_menu_mutes_the_thread_and_its_row_says_so() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor plan", 4, 10).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chips") == Some(serde_json::json!(4))
            )
            .await
        );
        let reading = window.reading().expect("open");
        support::click(&window, &thread_button(&reading), 1);
        assert!(
            crate::settle_until(
                async || said(&window, "focus.menu.open") == Some(serde_json::json!(true))
            )
            .await,
            "Thread opened the menu"
        );
        assert_eq!(
            said(&window, "focus.menu.groups"),
            Some(serde_json::json!("Whole thread | This message only"))
        );
        let mute = support::descendants(&thread_button(&reading))
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
            .find(|button| {
                button.has_css_class("focus-thread-menu-entry")
                    && support::texts(button)
                        .iter()
                        .any(|text| text == "Mute thread")
            })
            .expect("a Mute entry");
        // A menu's entries are choices, drawn in the window's ink: not the
        // grey a ghost button wears in the chrome, which made the menu read
        // as disabled.
        let subject = support::descendants(window.upcast_ref::<gtk::Widget>())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
            .find(|label| label.has_css_class("focus-open-subject"))
            .expect("the message's subject");
        assert_eq!(
            mute.color(),
            subject.color(),
            "a menu entry is drawn in the chrome's grey, not in ink"
        );
        mute.emit_clicked();
        assert!(
            crate::settle_until(
                async || said(&window, "focus.row.muted") == Some(serde_json::json!("muted"))
            )
            .await,
            "the row says muted: {:?}",
            said(&window, "focus.row.muted")
        );
        assert_eq!(
            said(&window, "focus.menu.open"),
            Some(serde_json::json!(false)),
            "picking closed the menu"
        );
    });
}

pub fn the_outline_filters_by_name_and_return_shows_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_with_you("Plans", 10).await;
        fixture.read_first(&ids, 1).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(1))
            )
            .await,
            "the window opened on the first unread, the account's own reply"
        );
        support::press(&window, "t", gtk::gdk::ModifierType::ALT_MASK);
        assert!(
            crate::settle_until(
                async || said(&window, "focus.outline.open") == Some(serde_json::json!(true))
            )
            .await,
            "alt+t opened the outline"
        );
        assert_eq!(
            said(&window, "focus.outline.rows"),
            Some(serde_json::json!(2))
        );
        let reading = window.reading().expect("open");
        // The outline is the width the design gives it, whole inside the
        // window, and in the interface's face (a popover is its own surface
        // and does not inherit the window's font).
        let outline = support::descendants(&thread_button(&reading))
            .into_iter()
            .find_map(|widget| {
                widget
                    .downcast::<gtk::Popover>()
                    .ok()
                    .filter(|popover| popover.has_css_class("focus-outline"))
            })
            .expect("the outline popover");
        let bounds = outline
            .child()
            .and_then(|body| body.compute_bounds(&window))
            .expect("the outline is placed in the window");
        assert!(
            bounds.width() <= 450.0 && bounds.x() + bounds.width() <= window.width() as f32,
            "the outline is 440 wide and inside the window: {bounds:?} in {}",
            window.width()
        );
        let face = outline
            .child()
            .and_then(|body| support::descendants(&body).into_iter().next_back())
            .map(|widget| widget.pango_context().font_description())
            .and_then(|font| font.and_then(|font| font.family()))
            .map(|family| family.to_string())
            .unwrap_or_default();
        assert!(
            !face.to_lowercase().contains("mono"),
            "the outline's names and lines are not in the mono face: {face:?}"
        );
        let field = support::descendants(&thread_button(&reading))
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
            .expect("the filter field");
        field.set_text("ada");
        assert!(
            crate::settle_until(
                async || said(&window, "focus.outline.rows") == Some(serde_json::json!(1))
            )
            .await,
            "typing a name left that person's messages"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(
                async || window.observe().reading.id == Some(ids[0].get().to_string())
            )
            .await,
            "Return showed Ada's message: {:?}",
            window.observe().reading.id
        );
        assert_eq!(
            said(&window, "focus.outline.open"),
            Some(serde_json::json!(false))
        );
        assert!(
            crate::settle_until(async || window
                .observe()
                .keyboard
                .widget
                .ends_with("GtkScrolledWindow"))
            .await,
            "the keyboard went back to the message, not the dialog's frame: {:?}",
            window.observe().keyboard
        );
    });
}

/// The Thread button in the message window's bar.
fn thread_button(reading: &postio_gtk::open::OpenMessage) -> gtk::Widget {
    support::only(&reading.dialog(), "focus-thread-bar-menu")
}

pub fn a_window_opened_from_a_search_walks_the_matches_and_escape_returns_to_the_child() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        for n in [3, 5, 6] {
            fixture
                .write_searchable_body(
                    ids[n - 1],
                    "Please publish the rate limit headers on every reply.",
                )
                .await;
        }
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("rate limit headers");
        assert!(
            crate::settle_until(async || bar
                .dropdown()
                .texts()
                .iter()
                .any(|text| text.contains("in the main window")))
            .await,
            "no Show all: {:?}",
            bar.dropdown().texts()
        );
        bar.run_search_row();
        assert!(crate::settle_until(async || window.results().is_some()).await);
        assert!(
            crate::settle_until(
                async || said(&window, "focus.results.children") == Some(serde_json::json!(4))
            )
            .await,
            "the top result opened in place: three matches and the gap between them: {:?}",
            said(&window, "focus.results.children")
        );
        support::keys(&window, &["j"]);
        assert_eq!(
            said(&window, "focus.results.child"),
            Some(serde_json::json!(0))
        );
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(2))
            )
            .await,
            "the window opened on message 3"
        );
        assert_eq!(
            reading_said(&window, "matched"),
            Some(serde_json::json!("3, 5, 6"))
        );
        assert!(
            reading_said(&window, "match_banner")
                .and_then(|text| text.as_str().map(str::to_owned))
                .is_some_and(|text| text.contains("match 1 of 3")),
            "the banner says which match: {:?}",
            reading_said(&window, "match_banner")
        );
        for chip in [4, 5] {
            support::press(&window, "g", gtk::gdk::ModifierType::CONTROL_MASK);
            assert!(
                crate::settle_until(
                    async || reading_said(&window, "chip") == Some(serde_json::json!(chip))
                )
                .await,
                "find carried the window to chip {chip}: {:?}",
                reading_said(&window, "chip")
            );
        }
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(
                async || said(&window, "focus.results.child") == Some(serde_json::json!(0))
            )
            .await,
            "Escape put the ring back on the child the window was opened from: {:?}",
            said(&window, "focus.results.child")
        );
    });
}

/// Ordinary find in a window opened from the list still wraps: only a window
/// a search opened stops at the ends and goes on to the next matched message.
pub fn find_in_a_window_opened_from_the_list_still_wraps() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Echo",
                "alpha beta alpha",
                10,
            )
            .await;
        fixture.write_body(message, "alpha beta alpha").await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || reading.body_text().contains("alpha")).await,
            "the body never drew"
        );
        let view = reading.reader().view().clone();
        view.set_find_query("alpha");
        assert!(
            crate::settle_until(async || view.find_state() == (Some(0), 2)).await,
            "two matches, the first current: {:?}",
            view.find_state()
        );
        support::press(&window, "g", gtk::gdk::ModifierType::CONTROL_MASK);
        assert_eq!(view.find_state(), (Some(1), 2));
        support::press(&window, "g", gtk::gdk::ModifierType::CONTROL_MASK);
        assert_eq!(
            view.find_state(),
            (Some(0), 2),
            "find wrapped from the last match to the first"
        );
    });
}

fn compose_said(window: &postio_gtk::window::FocusWindow, key: &str) -> Option<serde_json::Value> {
    said(window, &format!("focus.compose.{key}"))
}

pub fn a_reply_to_an_older_message_says_so_and_alt_e_keeps_what_was_typed() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        fixture.read_first(&ids, 5).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(5))
            )
            .await
        );
        for _ in 0..2 {
            support::press(&window, "bracketleft", gtk::gdk::ModifierType::empty());
        }
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(3))
            )
            .await
        );
        support::keys(&window, &["r"]);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "older.open") == Some(serde_json::json!(true))
            )
            .await,
            "the bar says this reply is to an older message"
        );
        assert_eq!(
            compose_said(&window, "older.title"),
            Some(serde_json::json!(
                "You\u{2019}re replying to message 4 of 6, not the latest"
            ))
        );
        assert!(
            crate::settle_until(
                async || compose_said(&window, "replying_to") == Some(serde_json::json!(4))
            )
            .await
        );
        let composer = window.composer().expect("the composer");
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "the reply opened"
        );
        // Typing into an editor whose document is still loading is lost:
        // wait until it holds the quote. (Where the keyboard lands is the
        // reply-older storyboard's question, asked of the real window.)
        assert!(
            crate::settle_until(async || composer
                .test_body_eval("document.body.innerText.includes('wrote')")
                == "true")
            .await,
            "the reply's body never loaded with its quote: {:?}",
            composer.test_body_eval("document.body.innerText")
        );
        // Typed at the caret, above the quote, as a person does.
        composer.test_body_eval("document.execCommand('insertText', false, 'Agreed on exports.')");
        assert!(
            crate::settle_until(async || composer
                .draft()
                .body
                .text
                .unwrap_or_default()
                .contains("Agreed on exports."))
            .await,
            "the words reached the draft"
        );
        support::press(&window, "e", gtk::gdk::ModifierType::ALT_MASK);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "replying_to") == Some(serde_json::json!(6))
            )
            .await,
            "alt+e moved the reply to message 6: {:?}",
            compose_said(&window, "replying_to")
        );
        assert_eq!(
            compose_said(&window, "older.open"),
            Some(serde_json::json!(false))
        );
        let body = composer.draft().body.text.unwrap_or_default();
        assert!(
            body.contains("Agreed on exports."),
            "what was typed is still there: {body:?}"
        );
    });
}

pub fn forwarding_the_whole_thread_says_so_in_the_subject() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor plan", 6, 10).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["f"]);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.open") == Some(serde_json::json!(true))
            )
            .await,
            "a forward of a thread offers the chooser"
        );
        assert_eq!(
            compose_said(&window, "chooser.selected"),
            Some(serde_json::json!("This message"))
        );
        support::keys(&window, &["2"]);
        assert!(
            crate::settle_until(async || compose_said(&window, "subject")
                == Some(serde_json::json!("Fwd: Harbor plan (6 messages)")))
            .await,
            "the subject says six messages: {:?}",
            compose_said(&window, "subject")
        );
        assert_eq!(
            compose_said(&window, "chooser.selected"),
            Some(serde_json::json!("Whole thread"))
        );
    });
}

/// An unsent reply is a child of its thread with no message of its own;
/// `Return` on it opens that draft in the composer, with its words.
pub fn return_on_the_draft_child_opens_the_draft_in_the_composer() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 3, 10).await;
        fixture
            .thread_draft(ids[2], "Thanks, I will check the exports tomorrow.")
            .await;
        let (window, _client) = fixture.open().await;
        let pane = window.pane().expect("the inbox");
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Right"]);
        // The row, three messages, and the draft.
        assert!(crate::settle_until(async || pane.cursor().n_items() == 5).await);
        support::keys(&window, &["j", "j", "j", "j"]);
        assert_eq!(pane.cursor().selected(), 4, "on the draft child");
        support::keys(&window, &["Return"]);
        let composer = window.composer().expect("the composer");
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "the draft opened in the composer"
        );
        assert!(
            crate::settle_until(async || composer
                .draft()
                .body
                .text
                .unwrap_or_default()
                .contains("check the exports tomorrow"))
            .await,
            "with its own words: {:?}",
            composer.draft().body.text
        );
        assert!(
            window.observe().reading.id.is_none(),
            "a draft is written, not read"
        );
    });
}

/// The chips drawn in the message window's thread bar, left to right.
fn chips_of(reading: &postio_gtk::open::OpenMessage) -> Vec<gtk::Widget> {
    support::with_class(&reading.dialog(), "focus-thread-chip")
        .into_iter()
        .filter(|chip| chip.root().is_some())
        .collect()
}

/// Click the `at`th chip wearing `class` once a click there reaches it:
/// the dialog is still opening for a moment, and the bar redraws its chips
/// when it is told how many fit, so the chip is found again on every try.
async fn click_chip(
    window: &postio_gtk::window::FocusWindow,
    reading: &postio_gtk::open::OpenMessage,
    class: &str,
    at: usize,
) {
    let target = || -> Option<(gtk::Widget, f64, f64)> {
        let chip = support::with_class(&reading.dialog(), class)
            .into_iter()
            .filter(|chip| chip.root().is_some())
            .nth(at)?;
        let bounds = chip.compute_bounds(window)?;
        let x = f64::from(bounds.x() + bounds.width() / 2.0);
        let y = f64::from(bounds.y() + bounds.height() / 2.0);
        let picked = window.pick(x, y, gtk::PickFlags::DEFAULT)?;
        (picked == chip || picked.is_ancestor(&chip)).then_some((chip, x, y))
    };
    assert!(
        crate::settle_until(async || target().is_some()).await,
        "a click reaches the chip"
    );
    let (_, x, y) = target().expect("the chip");
    support::click_at(window, x, y, 1);
}

pub fn clicking_a_thread_bar_chip_shows_that_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        fixture.read_first(&ids, 4).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(4))
            )
            .await
        );
        let reading = window.reading().expect("open");
        assert!(
            crate::settle_until(async || chips_of(&reading).len() == 6).await,
            "every member has a chip"
        );
        click_chip(&window, &reading, "focus-thread-chip", 1).await;
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(1))
            )
            .await,
            "the click showed message 2: {:?}",
            reading_said(&window, "chip")
        );
        assert_eq!(
            window.observe().reading.id,
            Some(ids[1].get().to_string()),
            "the second message is on screen"
        );
    });
}

pub fn clicking_the_more_chip_opens_the_outline() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 40, 10).await;
        fixture.read_first(&ids, 39).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || reading_said(&window, "chips")
                .and_then(|chips| chips.as_u64())
                .is_some_and(|chips| chips > 0))
            .await
        );
        let reading = window.reading().expect("open");
        let more = || {
            support::with_class(&reading.dialog(), "focus-thread-chip-more")
                .into_iter()
                .filter(|chip| chip.root().is_some())
                .collect::<Vec<_>>()
        };
        assert!(
            crate::settle_until(async || more().len() == 1).await,
            "forty messages do not all fit"
        );

        click_chip(&window, &reading, "focus-thread-chip-more", 0).await;
        assert!(
            crate::settle_until(
                async || said(&window, "focus.outline.open") == Some(serde_json::json!(true))
            )
            .await,
            "the click opened the outline"
        );
    });
}

/// A search result's child row says who wrote the matching message, with
/// their initials and the time, from what the controller sent with the row.
pub fn a_search_childs_row_says_who_wrote_it_and_when() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        for n in [3, 5, 6] {
            fixture
                .write_searchable_body(
                    ids[n - 1],
                    "Please publish the rate limit headers on every reply.",
                )
                .await;
        }
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("rate limit headers");
        assert!(
            crate::settle_until(async || bar
                .dropdown()
                .texts()
                .iter()
                .any(|text| text.contains("in the main window")))
            .await
        );
        bar.run_search_row();
        assert!(
            crate::settle_until(
                async || said(&window, "focus.results.children") == Some(serde_json::json!(4))
            )
            .await
        );
        let rows = || {
            support::with_class(&window, "focus-results-child")
                .iter()
                .map(support::texts)
                .filter(|texts| texts.iter().any(|text| text.starts_with("message ")))
                .collect::<Vec<_>>()
        };
        assert!(
            crate::settle_until(async || rows().len() == 3).await,
            "three matching children are drawn: {:?}",
            rows()
        );
        for row in rows() {
            assert!(row.contains(&"AM".to_owned()), "initials: {row:?}");
            assert!(row.contains(&"Ada Moreno".to_owned()), "sender: {row:?}");
            assert!(
                row.len() >= 5,
                "avatar, sender, tag, passage and time: {row:?}"
            );
        }
    });
}

/// The chooser takes the keyboard when it opens, so `1` `2` `3` are its own
/// there; Tab moves to To, where they are digits in an address again.
pub fn the_forward_chooser_answers_its_number_keys_only_while_it_has_the_keyboard() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor plan", 6, 10).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["f"]);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.open") == Some(serde_json::json!(true))
            )
            .await
        );
        let composer = window.composer().expect("the composer");
        assert!(
            crate::settle_until(async || composer.is_open()).await,
            "the forward opened"
        );
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.focused") == Some(serde_json::json!(true))
            )
            .await,
            "the keyboard is in the chooser: {:?}",
            support::focus_path(&window)
        );
        support::keys(&window, &["2"]);
        assert!(
            crate::settle_until(async || compose_said(&window, "chooser.selected")
                == Some(serde_json::json!("Whole thread")))
            .await,
            "2 chose Whole thread"
        );
        // A Tab is GTK's own traversal, which the harness's key delivery
        // cannot run, and imitating it with `child_focus` started from the
        // dialog's top on a loaded CI runner (PR #2124). This case asks what
        // the number keys do once To has the keyboard, so it puts the
        // keyboard there, as the Tab does.
        assert!(composer.test_focus_field(postio_widgets::composer::Field::To));
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.focused") == Some(serde_json::json!(false))
            )
            .await,
            "Tab left the chooser: {}",
            support::focus_path(&window)
        );
        // The field learns it has the keyboard from its own focus signal,
        // which a loaded machine delivers a beat after the chooser's.
        assert!(
            crate::settle_until(
                async || composer.focused_field() == Some(postio_widgets::composer::Field::To)
            )
            .await,
            "Tab moved to To: {}",
            support::focus_path(&window)
        );
        support::keys(&window, &["1"]);
        assert_eq!(
            compose_said(&window, "chooser.selected"),
            Some(serde_json::json!("Whole thread")),
            "1 in To is a character of an address, not the chooser's"
        );
    });
}

/// A forwarded thread sits under what is typed as one row a message, and
/// `mod+shift+q` opens them into the body and folds them again; what is
/// sent is the same either way.
pub fn mod_shift_q_folds_and_unfolds_the_forwarded_messages() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor plan", 6, 10).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["f"]);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.open") == Some(serde_json::json!(true))
            )
            .await
        );
        let composer = window.composer().expect("the composer");
        assert!(crate::settle_until(async || composer.is_open()).await);
        support::keys(&window, &["2"]);
        assert!(
            crate::settle_until(
                async || compose_said(&window, "forwarded.rows") == Some(serde_json::json!(6))
            )
            .await,
            "six messages fold to six rows: {:?}",
            compose_said(&window, "forwarded.rows")
        );
        // The rows are the text being forwarded: they sit in the composer
        // under what is typed, not below its attachments list like more
        // attachments.
        let dialog = window.compose_dialog().expect("the compose dialog");
        let rows = support::only(&dialog, "focus-forwarded");
        assert!(
            rows.is_ancestor(&composer),
            "the forwarded rows are inside the composer, under its body"
        );
        let seen = || composer.test_body_eval("document.body.innerText");
        assert!(
            !seen().contains("Forwarded messages"),
            "folded, the body shows only what is typed: {:?}",
            seen()
        );
        let sent = composer.draft().body.text.unwrap_or_default();
        assert!(
            sent.contains("Forwarded messages") && sent.contains("Message 6"),
            "and the draft still carries every message: {sent:?}"
        );

        support::press(
            &window,
            "q",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(
            crate::settle_until(
                async || compose_said(&window, "forwarded.rows") == Some(serde_json::json!(0))
            )
            .await,
            "the rows open into the body"
        );
        assert!(
            crate::settle_until(async || seen().contains("Forwarded messages")).await,
            "the messages are in the body: {:?}",
            seen()
        );
        assert_eq!(composer.draft().body.text.unwrap_or_default(), sent);

        support::press(
            &window,
            "q",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(
            crate::settle_until(
                async || compose_said(&window, "forwarded.rows") == Some(serde_json::json!(6))
            )
            .await,
            "and fold again"
        );
        assert_eq!(composer.draft().body.text.unwrap_or_default(), sent);
    });
}

/// Splitting the message on screen off its thread leaves it on screen, now
/// the first of its own thread, and the toast that offers Undo stays up
/// rather than giving way to a "that message is gone" notice.
pub fn splitting_the_shown_message_keeps_it_on_screen_in_its_new_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 6, 10).await;
        fixture.read_first(&ids, 4).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chip") == Some(serde_json::json!(4))
            )
            .await,
            "the window opened on message 5"
        );
        window.act(postio_core::CommandId::SplitThread);
        assert!(
            crate::settle_until(
                async || reading_said(&window, "chips") == Some(serde_json::json!(2))
            )
            .await,
            "the bar is the new thread's two messages: {:?}",
            reading_said(&window, "chips")
        );
        assert_eq!(
            window.observe().reading.id,
            Some(ids[4].get().to_string()),
            "the same message is on screen"
        );
        assert_eq!(
            reading_said(&window, "chip"),
            Some(serde_json::json!(0)),
            "now the first of its thread"
        );
        assert_eq!(
            reading_said(&window, "kicker").and_then(|kicker| kicker
                .as_str()
                .map(|kicker| kicker.starts_with("Message 1 of 2"))),
            Some(true),
            "{:?}",
            reading_said(&window, "kicker")
        );
        let toast = window.toast_showing().unwrap_or_default();
        assert!(
            toast.to_lowercase().contains("split") && !toast.contains("gone"),
            "the Undo toast stays: {toast:?}"
        );
    });
}

/// `alt+t` on a thread opened in place shows its outline from the list,
/// with no message window to hang it from (specs/013-focus-threads US5).
pub fn the_outline_opens_from_the_list_over_an_expanded_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _ids, pane) = harbor().await;
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::press(&window, "t", gtk::gdk::ModifierType::ALT_MASK);
        assert!(
            crate::settle_until(
                async || said(&window, "focus.outline.open") == Some(serde_json::json!(true))
            )
            .await,
            "alt+t opened no outline from the list"
        );
        assert_eq!(
            said(&window, "focus.outline.rows"),
            Some(serde_json::json!(4)),
            "the outline lists the four messages"
        );
    });
}

/// `alt+v` on a child row draws the move picker for that message alone, and
/// choosing a folder moves it and leaves the rest of its thread.
pub fn alt_v_on_a_child_moves_that_message_alone() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, ids, pane) = harbor().await;
        let receipts = fixture.folder("Receipts").await;
        support::keys(&window, &["Right"]);
        assert!(crate::settle_until(async || pane.cursor().n_items() == 6).await);
        support::keys(&window, &["j"]);
        support::press(&window, "v", gtk::gdk::ModifierType::ALT_MASK);
        assert!(
            crate::settle_until(async || window.open_picker().is_some()).await,
            "alt+v drew no move picker on a child"
        );
        let picker = window.open_picker().expect("open");
        assert!(
            crate::settle_until(async || picker.texts().iter().any(|line| line == "Receipts"))
                .await,
            "the folders never listed: {:?}",
            picker.texts()
        );
        let said = picker.texts();
        assert!(said.iter().any(|line| line == "Move to folder"), "{said:?}");
        support::keys(&window, &["Down", "Return"]);
        let moved = async |message: MessageId| {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::MessageRepository::new(&connection)
                .get(message)
                .await
                .expect("a read")
                .expect("the message")
                .mailbox_id
        };
        assert!(
            crate::settle_until(async || moved(ids[0]).await == receipts).await,
            "the child did not move"
        );
        for other in &ids[1..] {
            assert_ne!(
                moved(*other).await,
                receipts,
                "the rest of its thread stays where it was"
            );
        }
    });
}

/// How many times each child's line is drawn, in the order the rows are.
fn children_drawn(pane: &postio_gtk::list::ListPane) -> Vec<String> {
    drawn(pane)
        .into_iter()
        .filter(|text| text.starts_with("Message "))
        .collect()
}

/// New mail landing above an open thread moves its row down, and its
/// children go with it: each is drawn once, under the thread's row, and
/// `Left` folds all of them away (a sync while a thread was open left two
/// children behind on screen, 2026-11-16).
pub fn new_mail_above_an_open_thread_moves_its_children_with_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Harbor plan", 3, 60).await;
        fixture.file_in(fixture.inbox, "Top", 10).await;
        let (window, sink) = opened_telling(&fixture).await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 2).await,
            "the inbox opens with a message above the thread"
        );
        support::keys(&window, &["j", "Right"]);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 5).await,
            "Right spliced the three messages in: {} rows",
            pane.cursor().n_items()
        );

        let fresh = fixture.file_in(fixture.inbox, "Fresh", 1).await;
        sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![fresh],
        });
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 6
                && support::subjects(&window).first().map(String::as_str) == Some("Fresh"))
            .await,
            "the new mail landed at the top: {:?}",
            support::subjects(&window)
        );
        // The rows the splice moved are laid out a frame or two after the
        // count changes: wait for them to be drawn, then hold them to it.
        let children = ["Message 1", "Message 2", "Message 3"];
        assert!(
            crate::settle_until(async || children_drawn(&pane) == children).await,
            "each child is drawn once: {:?}",
            children_drawn(&pane)
        );
        crate::settle();
        assert_eq!(children_drawn(&pane), children, "and stays drawn once");
        let thread_row = pane
            .cursor()
            .item(2)
            .and_downcast::<postio_gtk::list::model::RowObject>()
            .and_then(|row| row.child_view());
        assert!(
            thread_row.is_none(),
            "the thread's row is third, under the new mail and Top, not a child"
        );

        support::keys(&window, &["Left"]);
        assert!(
            crate::settle_until(async || pane.cursor().n_items() == 3).await,
            "Left folded the children: {} rows",
            pane.cursor().n_items()
        );
        crate::settle();
        assert_eq!(
            children_drawn(&pane),
            Vec::<String>::new(),
            "no child is left on screen"
        );
    });
}

/// The cursor's row's thread and what its row draws, once drawn.
fn cursor_thread(window: &postio_gtk::window::FocusWindow) -> Option<postio_model::ThreadId> {
    window.cursor_row().and_then(|row| row.thread())
}

/// A reply to the thread under the cursor, while its message is open in a
/// window: the cursor behind the window stays on the thread's row, which
/// now names who replied first and says their line, and Escape comes back
/// to it (#2107; the cursor slid to the row below, and the row kept the
/// sender and line it had).
pub fn a_reply_keeps_the_cursor_on_its_thread_and_the_row_says_the_reply() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let ids = fixture.thread_of("Harbor plan", 3, 10).await;
        fixture.file_in(fixture.inbox, "Solo", 300).await;
        let (window, sink) = opened_telling(&fixture).await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(
                async || pane.cursor().selected() == 0 && pane.cursor().n_items() == 2
            )
            .await,
            "the inbox opens on the thread, above the single message"
        );
        let harbor = cursor_thread(&window).expect("the cursor is on the thread");
        let before = first_row(&pane).await;
        assert!(
            before.iter().any(|text| text.contains("Message 3")),
            "the row says the thread's newest line before the reply: {before:?}"
        );
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || window.observe().reading.id.is_some()).await,
            "Return opened the thread's message in a window"
        );

        let (reply, thread) = fixture.reply_to(*ids.last().unwrap(), "Exports too").await;
        assert_eq!(thread, harbor, "the reply joined the thread");
        sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![reply],
        });
        sink.emit(postio_core::Event::ThreadChanged {
            account: fixture.account.id,
            thread,
            members_added: vec![reply],
        });
        assert!(
            crate::settle_until(async || first_row(&pane)
                .await
                .iter()
                .any(|text| text.contains("Exports too")))
            .await,
            "the row never said the reply's line: {:?}",
            first_row(&pane).await
        );
        crate::settle();
        let after = first_row(&pane).await;
        assert_eq!(
            after.first().map(String::as_str),
            Some("Ana, Ada"),
            "the row names who replied first: {after:?}"
        );
        assert_eq!(
            (pane.cursor().selected(), cursor_thread(&window)),
            (0, Some(harbor)),
            "the cursor stays on the thread's row behind the window: {:?}",
            support::subjects(&window)
        );

        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.observe().reading.id.is_none()).await,
            "Escape closed the window"
        );
        assert_eq!(
            cursor_thread(&window),
            Some(harbor),
            "Escape came back to the thread's row: {:?}",
            support::subjects(&window)
        );
    });
}

/// A reply to the thread under the cursor further down the list moves its
/// row to the top, newest first, and the cursor goes with it (#2107): it
/// does not stay on the row that slid into the thread's old place.
pub fn a_reply_moves_its_thread_to_the_top_and_the_cursor_with_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.file_in(fixture.inbox, "Top", 5).await;
        let ids = fixture.thread_of("Harbor plan", 3, 60).await;
        fixture.file_in(fixture.inbox, "Solo", 300).await;
        let (window, sink) = opened_telling(&fixture).await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(
                async || pane.cursor().selected() == 0 && pane.cursor().n_items() == 3
            )
            .await,
            "the inbox opens on Top, above the thread and Solo"
        );
        support::keys(&window, &["j"]);
        let harbor = cursor_thread(&window).expect("the cursor is on the thread");

        let (reply, thread) = fixture.reply_to(*ids.last().unwrap(), "Exports too").await;
        sink.emit(postio_core::Event::NewMail {
            account: fixture.account.id,
            mailbox: fixture.inbox,
            messages: vec![reply],
        });
        sink.emit(postio_core::Event::ThreadChanged {
            account: fixture.account.id,
            thread,
            members_added: vec![reply],
        });
        assert!(
            crate::settle_until(async || pane
                .feed()
                .shown()
                .item(0)
                .and_downcast::<postio_gtk::list::model::RowObject>()
                .and_then(|row| row.item())
                .and_then(|row| row.thread())
                == Some(harbor))
            .await,
            "the thread's row never reached the top: {:?}",
            support::subjects(&window)
        );
        crate::settle();
        assert_eq!(
            (pane.cursor().selected(), cursor_thread(&window)),
            (0, Some(harbor)),
            "the cursor went with the thread: {:?}",
            support::subjects(&window)
        );
    });
}
