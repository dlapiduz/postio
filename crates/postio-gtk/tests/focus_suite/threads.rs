use crate::support::{self, Fixture};
use gtk::prelude::*;
use postio_model::MessageId;

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

fn drawn(pane: &postio_gtk::list::ListPane) -> Vec<String> {
    pane.rows_on_screen()
        .iter()
        .flat_map(|row| row.drawn().texts)
        .collect()
}

pub fn right_opens_a_thread_in_place() {
    crate::gtk_case(async {
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

pub fn clicking_the_chevron_opens_and_folds_a_thread() {
    crate::gtk_case(async {
        let (_fixture, window, _ids, pane) = harbor().await;
        let click = |window: &postio_gtk::window::FocusWindow| {
            let row = pane.rows_on_screen()[0].clone();
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

fn reading_said(window: &postio_gtk::window::FocusWindow, key: &str) -> Option<serde_json::Value> {
    said(window, &format!("focus.reading.{key}"))
}

pub fn the_window_walks_its_thread() {
    crate::gtk_case(async {
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

fn compose_said(window: &postio_gtk::window::FocusWindow, key: &str) -> Option<serde_json::Value> {
    said(window, &format!("focus.compose.{key}"))
}

fn chips_of(reading: &postio_gtk::open::OpenMessage) -> Vec<gtk::Widget> {
    support::with_class(&reading.dialog(), "focus-thread-chip")
        .into_iter()
        .filter(|chip| chip.root().is_some())
        .collect()
}

/// The dialog is still opening for a moment, and the bar redraws its chips when told how many fit,
/// so the chip is found again on every try.
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

pub fn the_outline_keeps_its_width_however_many_people_wrote() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let authors = [
            ("Ada Moreno-Castellanos", "ada@example.com"),
            ("Benedict Adeyemi-Okafor", "ben@example.com"),
            ("Carlota Fitzgerald-Whitmore", "carlota@example.com"),
            ("Dmitri Vasquez-Lindqvist", "dmitri@example.com"),
            ("Ada Moreno-Castellanos", "ada@example.com"),
        ];
        fixture.thread_by("Harbor plan", &authors, 10).await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || window.cursor_row().is_some()).await);
        support::keys(&window, &["Return"]);
        assert!(crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await);
        support::press(&window, "t", gtk::gdk::ModifierType::ALT_MASK);
        assert!(
            crate::settle_until(
                async || said(&window, "focus.outline.open") == Some(serde_json::json!(true))
            )
            .await,
            "alt+t opened the outline"
        );
        let reading = window.reading().expect("open");
        let outline = support::only(&reading.dialog(), "focus-outline");
        assert!(
            crate::settle_until(async || outline.width() > 0).await,
            "the outline was laid out"
        );
        assert_eq!(
            support::with_class(&outline, "focus-outline-chip").len(),
            4,
            "a chip for each person"
        );
        let shown = outline.first_child().expect("the popover's box");
        assert_eq!(
            shown.width(),
            440,
            "four people's chips widened the outline past its rule"
        );
    });
}

pub fn a_search_childs_row_says_who_wrote_it_and_when() {
    crate::gtk_case(async {
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
                async || said(&window, "focus.results.children") == Some(serde_json::json!(5))
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

pub fn the_forward_chooser_answers_its_number_keys_only_while_it_has_the_keyboard() {
    crate::gtk_case(async {
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
        // A Tab is GTK's own traversal, which the harness cannot run; imitating it with
        // `child_focus` started from the dialog's top on a loaded runner (#2124).
        // This case asks what the number keys do once To has the keyboard.
        assert!(composer.test_focus_field(postio_widgets::composer::Field::To));
        assert!(
            crate::settle_until(
                async || compose_said(&window, "chooser.focused") == Some(serde_json::json!(false))
            )
            .await,
            "Tab left the chooser: {}",
            support::focus_path(&window)
        );
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
