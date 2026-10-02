//! The open message beside the list (T232; screens.md, "Reading beside the
//! list"): `F8` and `[focus] reading` switch where `Return` opens a
//! message, the pane is the dialog's one message view placed beside the
//! list, `j`/`k` step it, `Escape` gives the keyboard back to the list, a
//! window under 980 px falls back to the dialog, and the composer takes the
//! pane over. Each measured on what GTK put on screen.

use adw::prelude::*;
use gtk::gdk;
use postio_ui::focus_dialog;

use crate::support::{self, Fixture};

/// The one widget wearing `class` under `root`, if there is one on screen.
fn on_screen(root: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    support::with_class(root, class)
        .into_iter()
        .find(|widget| widget.is_mapped())
}

/// Whether the message dialog is over the window.
fn dialog_up(window: &postio_focus::window::FocusWindow) -> bool {
    window
        .visible_dialog()
        .is_some_and(|dialog| dialog.widget_name() == postio_focus::open::DIALOG_NAME)
}

/// The pane, on screen.
fn pane(window: &postio_focus::window::FocusWindow) -> gtk::Widget {
    window
        .reading_pane()
        .filter(|pane| pane.is_mapped())
        .expect("the reading pane is on screen")
}

/// The open message's header title, as drawn in the pane.
fn pane_title(window: &postio_focus::window::FocusWindow) -> Option<String> {
    let pane = window.reading_pane()?;
    let title = on_screen(&pane, "focus-open-title")?;
    Some(title.downcast::<gtk::Label>().ok()?.text().to_string())
}

/// Put `window` in pane mode as the setting does: what the config watcher
/// hands the window when `[focus] reading = "pane"` is saved.
fn read_in_pane(window: &postio_focus::window::FocusWindow) {
    window.set_focus_config(postio_config::FocusConfig {
        reading: postio_config::Reading::Pane,
        ..postio_config::FocusConfig::default()
    });
    crate::settle();
}

/// The list's cursor, as a row index.
fn cursor(window: &postio_focus::window::FocusWindow) -> u32 {
    window.pane().expect("the list").cursor().selected()
}

/// `F8` and the setting each switch where `Return` opens a message: the
/// dialog by default, the pane once switched, the dialog again after; and
/// `F8` writes its choice to `config.toml`.
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

        // The default: the dialog.
        support::deliver(&window, "j");
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || dialog_up(&window)).await,
            "Return opened no dialog by default"
        );
        support::deliver(&window, "Escape");
        assert!(crate::settle_until(async || !dialog_up(&window)).await);

        // F8: beside the list, and said in the file.
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

        // F8 again, with the message open: it moves to the dialog, still open.
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

        // The setting, as the config watcher hands it over.
        read_in_pane(&window);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).is_some()).await,
            "[focus] reading = \"pane\" did not open the pane"
        );
        assert!(!dialog_up(&window));
    });
}

/// `Return` in pane mode shows the message beside the list: the list is
/// still on screen at its width, its cursor where it was, and the pane is
/// `pane_width` wide at its right.
pub fn return_shows_the_message_beside_the_list_and_keeps_its_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j", "j", "j"]);
        assert_eq!(cursor(&window), 2);
        support::deliver(&window, "Return");
        assert!(
            crate::settle_until(async || pane_title(&window).as_deref() == Some("Third")).await,
            "the pane never showed the cursor's row"
        );
        crate::settle();
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

/// `j` and `k` step the pane: the cursor moves, and the pane shows its row.
pub fn j_and_k_step_the_pane_with_the_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        read_in_pane(&window);
        support::keys(&window, &["j", "j", "j"]);
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

/// `Escape` and the pane's X each close the message and give the keyboard
/// back to the list, on the row it was on; the pane says nothing is open and
/// how to open something.
pub fn escape_and_the_x_return_the_keyboard_to_the_list() {
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
        // And the keyboard works there: j moves the cursor, opening nothing.
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

/// A window under 980 px has no room for the pane: `Return` opens the
/// dialog, whatever the setting says, and the pane is not drawn.
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

/// The column inside the pane is `column_width` for the pane's width and
/// the body's treatment: 480 in the app's colours.
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
            // Centred in the pane.
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

/// Reply in pane mode puts the composer where the message was, beside the
/// list and in no dialog; `Escape` gives the pane back to the message.
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

        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || {
                window.reading_pane().is_some_and(|pane| {
                    on_screen(&pane, "focus-compose-surface").is_some()
                        && support::texts(&pane).iter().any(|text| text == "Reply")
                })
            })
            .await,
            "e did not put the composer in the pane: {:?}",
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
        // Its column is the pane's, in the app's colours.
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
