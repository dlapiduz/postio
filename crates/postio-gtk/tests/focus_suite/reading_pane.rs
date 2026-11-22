use adw::prelude::*;
use gtk::gdk;
use postio_ui::focus_dialog;

use crate::support::{self, Fixture};

fn on_screen(root: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    support::with_class(root, class)
        .into_iter()
        .find(|widget| widget.is_mapped())
}

fn dialog_up(window: &postio_gtk::window::FocusWindow) -> bool {
    window
        .visible_dialog()
        .is_some_and(|dialog| dialog.widget_name() == postio_gtk::open::DIALOG_NAME)
}

fn pane(window: &postio_gtk::window::FocusWindow) -> gtk::Widget {
    window
        .reading_pane()
        .filter(|pane| pane.is_mapped())
        .expect("the reading pane is on screen")
}

fn pane_title(window: &postio_gtk::window::FocusWindow) -> Option<String> {
    let pane = window.reading_pane()?;
    let title = on_screen(&pane, "focus-open-title")?;
    Some(title.downcast::<gtk::Label>().ok()?.text().to_string())
}

fn read_in_pane(window: &postio_gtk::window::FocusWindow) {
    window.set_focus_config(postio_config::FocusConfig {
        reading: postio_config::Reading::Pane,
        ..postio_config::FocusConfig::default()
    });
    crate::settle();
}

async fn pane_laid_out(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || {
            let expected = focus_dialog::pane_width(window.width());
            window.reading_pane().is_some_and(|pane| {
                pane.is_mapped()
                    && expected.is_some_and(|expected| (pane.width() - expected).abs() <= 1)
            })
        })
        .await,
        "the pane never took its width: {:?} in a {} window",
        window.reading_pane().map(|pane| pane.width()),
        window.width()
    );
}

fn cursor(window: &postio_gtk::window::FocusWindow) -> u32 {
    window.pane().expect("the list").cursor().selected()
}

pub fn f8_and_the_setting_switch_between_the_dialog_and_the_pane() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let dir = tempfile::tempdir().expect("a config directory");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "# Mine.\n").expect("a config file");
        window.set_config_path(Some(config.clone()));

        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || dialog_up(&window)).await,
            "Return opened no dialog by default"
        );
        support::deliver(&window, "Escape");
        assert!(crate::settle_until(async || !dialog_up(&window)).await);

        assert!(support::deliver(&window, "F8"), "F8 reached nothing");
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("First")).await,
            "Return did not open the message in the pane: {:?}",
            window.reading_pane().map(|pane| support::texts(&pane))
        );
        assert!(!dialog_up(&window), "a dialog opened as well");
        let written = std::fs::read_to_string(&config).expect("the file");
        assert!(
            written.starts_with("# Mine.\n") && written.contains("reading = \"pane\""),
            "F8 wrote its choice: {written:?}"
        );

        support::deliver(&window, "F8");
        assert!(
            crate::settle_until(async || dialog_up(&window)).await,
            "the open message did not move to the dialog"
        );
        assert_eq!(
            window.reading().map(|reading| reading.title()).as_deref(),
            Some("First")
        );
        assert!(
            window.reading_pane().is_none_or(|pane| !pane.is_mapped()),
            "the pane stayed beside the list"
        );
        support::deliver(&window, "Escape");
        assert!(crate::settle_until(async || !dialog_up(&window)).await);

        read_in_pane(&window);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).is_some()).await,
            "[focus] reading = \"pane\" did not open the pane"
        );
        assert!(!dialog_up(&window));
    });
}

pub fn return_shows_the_message_beside_the_list_and_keeps_its_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j", "j"]);
        assert_eq!(cursor(&window), 2);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Third")).await,
            "the pane never showed the cursor's row"
        );
        pane_laid_out(&window).await;
        let pane = pane(&window);
        let list = window.pane().expect("the list").view().clone();
        assert!(list.is_mapped(), "the list left the screen");
        assert_eq!(support::subjects(&window).len(), 5, "every row still shows");
        assert_eq!(cursor(&window), 2, "the cursor stayed on its row");
        let expected = focus_dialog::pane_width(window.width()).expect("room for a pane");
        let pane_bounds = pane.compute_bounds(&window).expect("laid out");
        let list_bounds = list.compute_bounds(&window).expect("laid out");
        assert!(
            (pane_bounds.width() - expected as f32).abs() <= 1.0,
            "the pane is {} wide in a {} window, not {expected}",
            pane_bounds.width(),
            window.width()
        );
        assert!(
            (pane_bounds.x() + pane_bounds.width() - window.width() as f32).abs() <= 1.0,
            "the pane is at the window's right"
        );
        assert!(
            list_bounds.x() + list_bounds.width() <= pane_bounds.x() + 0.5,
            "the list is beside the pane, not under it"
        );
        assert!(
            list_bounds.width() >= focus_dialog::LIST_MIN as f32 - 1.0,
            "the list keeps its floor: {}",
            list_bounds.width()
        );
    });
}

pub fn j_and_k_step_the_pane_with_the_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j", "j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Third")).await
        );
        support::deliver(&window, "j");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Fourth")).await,
            "j did not step the pane: {:?}",
            pane_title(&window)
        );
        assert_eq!(cursor(&window), 3);
        support::deliver(&window, "k");
        support::deliver(&window, "k");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await,
            "k did not step the pane back: {:?}",
            pane_title(&window)
        );
        assert_eq!(cursor(&window), 1);
        assert!(!dialog_up(&window), "stepping opened a dialog");
    });
}

pub fn escape_and_the_x_return_the_keyboard_to_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await
        );
        let list = window.pane().expect("the list").view().clone();
        let keyboard_in_list = || {
            gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus| {
                focus == *list.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&list)
            })
        };

        support::deliver(&window, "Escape");
        assert!(
            crate::settle_until(async || pane_title(&window).is_none()).await,
            "Escape left the message in the pane"
        );
        let said = support::texts(&pane(&window));
        assert!(
            said.iter().any(|text| text == "No message open"),
            "the empty pane says so: {said:?}"
        );
        assert!(
            keyboard_in_list(),
            "the keyboard is not in the list: {}",
            support::focus_path(&window)
        );
        assert_eq!(cursor(&window), 1, "the cursor stayed");
        support::deliver(&window, "j");
        assert_eq!(cursor(&window), 2);
        assert!(
            pane_title(&window).is_none(),
            "moving the cursor opened a message"
        );

        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Third")).await
        );
        pane_laid_out(&window).await;
        let close = on_screen(&pane(&window), "focus-open-close").expect("the pane's X");
        support::click(&window, &close, 1);
        assert!(
            crate::settle_until(async || pane_title(&window).is_none()).await,
            "the X left the message in the pane"
        );
        assert!(
            keyboard_in_list(),
            "the keyboard is not in the list: {}",
            support::focus_path(&window)
        );
        assert_eq!(cursor(&window), 2);
    });
}

pub fn a_window_under_980_opens_the_dialog_instead() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.file_five().await;
        let (window, _client) = fixture.open_sized(Some((960, 700))).await;
        window.unmaximize();
        assert!(
            crate::settle_until(async || window.width() == 960).await,
            "the window never came to 960 wide: {}",
            window.width()
        );
        read_in_pane(&window);
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || dialog_up(&window)).await,
            "a 960 px window did not fall back to the dialog"
        );
        assert!(
            window.reading_pane().is_none_or(|pane| !pane.is_mapped()),
            "the pane is drawn in a window with no room for it"
        );
    });
}

pub fn the_panes_column_follows_column_width_for_its_width() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for (minutes, subject) in [(10, "Plain"), (20, "Older")] {
            let (message, _) = fixture
                .file(("Ada Moreno", "ada@example.com"), subject, "x", minutes)
                .await;
            fixture.write_body(message, "A body of plain words.").await;
        }
        for size in [None, Some((1024, 700))] {
            let (window, _client) = fixture.open_sized(size).await;
            if let Some((width, _)) = size {
                window.unmaximize();
                assert!(crate::settle_until(async || window.width() == width).await);
            }
            read_in_pane(&window);
            support::keys(&window, &["j"]);
            support::deliver(&window, "Return");
            let reading = window.reading().expect("Return opened the message");
            assert!(
                crate::settle_until(async || reading.reader().view().document().is_some()).await,
                "the body was never drawn"
            );
            crate::settle();
            let pane_width = focus_dialog::pane_width(window.width()).expect("a pane");
            let expected = focus_dialog::column_width(pane_width, reading.reader().treatment());
            let column = on_screen(&pane(&window), "focus-open-column").expect("the column");
            assert!(
                crate::settle_until(async || column.width() == expected).await,
                "a {} window's pane ({pane_width}) holds a {} column, not {expected}",
                window.width(),
                column.width()
            );
            let pane_bounds = pane(&window).compute_bounds(&window).expect("laid out");
            let bounds = column.compute_bounds(&window).expect("laid out");
            let left = bounds.x() - pane_bounds.x();
            let right = pane_bounds.x() + pane_bounds.width() - (bounds.x() + bounds.width());
            assert!(
                (left - right).abs() <= 2.0,
                "off centre: {left} and {right}"
            );
            window.close();
            crate::settle();
        }
    });
}

pub fn the_composer_takes_over_the_pane() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let message = fixture.harbor_thread().await;
        let (window, _client) = fixture.open().await;
        read_in_pane(&window);
        support::keys(&window, &["j"]);
        assert_eq!(window.cursor_row().map(|row| row.id()), Some(message));
        support::deliver(&window, "Return");
        assert!(crate::settle_until(async || pane_title(&window).is_some()).await);

        support::keys(&window, &["r"]);
        assert!(
            crate::settle_until(async || {
                window.reading_pane().is_some_and(|pane| {
                    on_screen(&pane, "focus-compose-surface").is_some()
                        && support::texts(&pane).iter().any(|text| text == "Reply")
                })
            })
            .await,
            "r did not put the composer in the pane: {:?}",
            window.reading_pane().map(|pane| support::texts(&pane))
        );
        assert!(
            window.compose_dialog().is_none(),
            "the composer opened a dialog"
        );
        assert!(
            window.visible_dialog().is_none(),
            "something is over the window"
        );
        assert!(
            window.pane().expect("the list").view().is_mapped(),
            "the list left the screen"
        );
        let pane_width = focus_dialog::pane_width(window.width()).expect("a pane");
        let column = on_screen(&pane(&window), "focus-compose-column")
            .and_then(|clamp| clamp.first_child())
            .expect("the column");
        assert!(
            crate::settle_until(async || column.width()
                == focus_dialog::column_width(
                    pane_width,
                    postio_body::treatment::Treatment::AppColours
                ))
            .await,
            "the composer's column is {}",
            column.width()
        );

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                window
                    .reading_pane()
                    .is_some_and(|pane| on_screen(&pane, "focus-compose-surface").is_none())
                    && pane_title(&window).is_some()
            })
            .await,
            "Escape did not give the pane back to the message"
        );
    });
}

pub fn the_main_menus_check_item_is_the_pointers_f8() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let menu = support::only(&window, "focus-menu")
            .downcast::<gtk::MenuButton>()
            .expect("the main menu");
        let model = menu.menu_model().expect("the menu has items");
        let action = (0..model.n_items())
            .find(|item| {
                model
                    .item_attribute_value(*item, "label", Some(gtk::glib::VariantTy::STRING))
                    .and_then(|label| label.get::<String>())
                    .as_deref()
                    == Some("Read beside the list")
            })
            .and_then(|item| {
                model.item_attribute_value(item, "action", Some(gtk::glib::VariantTy::STRING))
            })
            .and_then(|action| action.get::<String>())
            .expect("the item names an action");
        let name = action.trim_start_matches("win.");
        let checked = || {
            window
                .action_state(name)
                .and_then(|state| state.get::<bool>())
        };
        assert_eq!(
            checked(),
            Some(false),
            "unchecked while messages open over the list"
        );

        gtk::prelude::WidgetExt::activate_action(&window, &action, None).expect("the item runs");
        crate::settle();
        assert_eq!(
            checked(),
            Some(true),
            "checked once messages open beside the list"
        );
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("First")).await,
            "the menu item did not switch to the pane"
        );

        window.set_focus_config(postio_config::FocusConfig::default());
        crate::settle();
        assert_eq!(checked(), Some(false), "the check follows the setting");
    });
}

pub fn a_click_moves_the_cursor_and_the_open_pane_follows_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        let list = window.pane().expect("the list");

        support::click(&window, &list.rows_on_screen()[3], 1);
        assert!(crate::settle_until(async || cursor(&window) == 3).await);
        crate::settle();
        assert!(pane_title(&window).is_none(), "a click opened a message");

        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Fourth")).await
        );
        support::click(&window, &list.rows_on_screen()[1], 1);
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await,
            "the pane did not follow the click: {:?}",
            pane_title(&window)
        );
        assert_eq!(cursor(&window), 1);
    });
}

pub fn archiving_steps_the_pane_past_the_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await
        );
        support::deliver(&window, "e");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Third")).await,
            "the pane did not step past the archived message: {:?}",
            pane_title(&window)
        );
        assert!(crate::settle_until(async || support::subjects(&window).len() == 4).await);
        assert_eq!(
            cursor(&window),
            1,
            "the cursor is on the row that took its place"
        );

        support::keys(&window, &["G"]);
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Fifth")).await,
            "G moved the cursor without the pane: {:?}",
            pane_title(&window)
        );
        support::deliver(&window, "e");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Fourth")).await,
            "the last row archived did not leave the pane on the previous one: {:?}",
            pane_title(&window)
        );
    });
}

/// A key pressed while the archive settles keeps its cursor: the step to the next message must not
/// drag it back to the row that took the archived one's place.
pub fn a_key_pressed_while_archiving_settles_keeps_its_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await
        );
        support::deliver(&window, "e");
        support::deliver(&window, "G");
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "the archive never left the list"
        );
        for _ in 0..30 {
            crate::settle();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(cursor(&window), 3, "the archive pulled the cursor back");
        assert_eq!(
            pane_title(&window).as_deref(),
            Some("Fifth"),
            "the pane did not stay with the cursor"
        );
    });
}

pub fn a_message_opened_away_from_the_inbox_opens_in_the_dialog() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.file_five().await;
        fixture
            .filtered(
                ("Forge", "noreply@forge.test"),
                "Build 1182 passed",
                "notification",
                Some("Forge"),
                30,
            )
            .await;
        let (window, _client) = fixture.open_five().await;
        read_in_pane(&window);
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || window
                .filtered()
                .is_some_and(|view| view.subjects() == ["Build 1182 passed"]))
            .await,
            "g f did not list the filtered message"
        );
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || dialog_up(&window)).await,
            "a message opened from Filtered did not open in the dialog"
        );
        assert_eq!(
            window.reading().map(|reading| reading.title()).as_deref(),
            Some("Build 1182 passed")
        );
        support::deliver(&window, "Escape");
        assert!(crate::settle_until(async || !dialog_up(&window)).await);

        support::keys(&window, &["g", "i"]);
        assert!(crate::settle_until(async || window.filtered().is_none()).await);
        support::keys(&window, &["j"]);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).is_some()).await,
            "back in the inbox, Return did not open beside the list"
        );
        assert!(!dialog_up(&window));
    });
}

/// `!` narrows the list under an open pane; the cursor lands on the narrowed list's first row and
/// the pane follows it, as for `j`.
pub fn a_filter_landing_under_an_open_pane_shows_the_cursors_row_there() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let filed = fixture.file_five().await;
        fixture.ask(filed[1], "Can you look at this?").await;
        fixture.ask(filed[3], "Could you sign it by Friday?").await;
        let (window, _client) = fixture.open_five().await;
        read_in_pane(&window);
        pane_laid_out(&window).await;
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("First")).await,
            "Return did not open First in the pane: {:?}",
            window.reading_pane().map(|pane| support::texts(&pane))
        );

        support::keys(&window, &["exclam"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Second", "Fourth"]).await,
            "only the marked rows remain: {:?}",
            support::subjects(&window)
        );
        crate::settle();
        assert_eq!(cursor(&window), 0, "the cursor is on the first marked row");
        assert_eq!(
            support::subjects(&window).first().map(String::as_str),
            Some("Second")
        );
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Second")).await,
            "the pane did not follow the cursor to Second: {:?}",
            pane_title(&window)
        );
    });
}

/// Eleven steps in one turn of the loop are one body read (#1609). Counted at the storage seam,
/// which sees the store owner's thread too.
pub fn a_sweep_reads_the_row_it_lands_on_not_every_row_it_passes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for at in 0..12 {
            let (message, _) = fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    &format!("Message {at:02}"),
                    "x",
                    10 + i64::from(at),
                )
                .await;
            fixture
                .write_body(message, &format!("Body number {at:02}."))
                .await;
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 12).await,
            "the inbox never listed its mail: {:?}",
            support::subjects(&window)
        );
        read_in_pane(&window);
        support::deliver(&window, "Return");
        let reading = window.reading().expect("Return opened the message");
        assert!(
            crate::settle_until(async || reading.body_text().contains("Body number 00")).await,
            "the first body was never drawn: {:?}",
            reading.body_text()
        );
        crate::settle();

        let loads = postio_storage::test_support::counting::body_loads();
        for _ in 0..11 {
            window.act(postio_core::CommandId::NextMessage);
        }
        assert!(
            crate::settle_until(async || reading.body_text().contains("Body number 11")).await,
            "the pane never showed where the cursor landed: {:?} {:?}",
            pane_title(&window),
            reading.body_text()
        );
        assert_eq!(cursor(&window), 11);
        assert_eq!(pane_title(&window).as_deref(), Some("Message 11"));
        let swept = postio_storage::test_support::counting::body_loads() - loads;
        assert_eq!(
            swept, 1,
            "eleven steps read {swept} bodies: every row the cursor passed, \
             not the one it landed on"
        );

        let loads = postio_storage::test_support::counting::body_loads();
        support::deliver(&window, "j");
        crate::settle();
        assert_eq!(cursor(&window), 11);
        assert_eq!(
            postio_storage::test_support::counting::body_loads() - loads,
            0,
            "a step that moved nothing read the body again"
        );
    });
}

pub fn opening_a_conversation_costs_the_same_however_long_it_is() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let mut costs = Vec::new();
        for count in [2, 12] {
            let fixture = Fixture::empty().await;
            let ids = fixture.thread_of("Harbor draft", count, 20).await;
            for id in &ids {
                fixture.write_body(*id, "The harbour schedule.").await;
            }
            let (window, _client) = fixture.open().await;
            assert!(
                crate::settle_until(async || support::subjects(&window).len() == 1).await,
                "the conversation was never listed: {:?}",
                support::subjects(&window)
            );
            crate::settle();
            let connections = postio_storage::test_support::counting::checkouts();
            let bodies = postio_storage::test_support::counting::body_loads();
            support::deliver(&window, "Return");
            assert!(
                crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
                "Return opened nothing"
            );
            let reading = window.reading().expect("open");
            assert!(
                crate::settle_until(async || {
                    reading.thread_known() == count
                        && reading.body_text().contains("harbour schedule")
                })
                .await,
                "the conversation never opened whole: {} of {count} known, {:?}",
                reading.thread_known(),
                reading.body_text()
            );
            crate::settle();
            costs.push((
                postio_storage::test_support::counting::checkouts() - connections,
                postio_storage::test_support::counting::body_loads() - bodies,
            ));
            window.close();
            crate::settle();
        }
        assert_eq!(
            costs[0], costs[1],
            "(connections, bodies) for two messages and for twelve: the open \
             grew with the conversation"
        );
        assert_eq!(costs[1].1, 1, "the open message's body, read once");
    });
}
