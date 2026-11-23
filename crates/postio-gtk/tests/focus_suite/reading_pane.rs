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

pub fn a_window_under_980_opens_the_dialog_instead() {
    crate::gtk_case(async {
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
        let fixture = Fixture::empty().await;
        for (minutes, subject) in [(10, "Plain"), (20, "Older")] {
            let (message, _) = fixture.file_from_ada(subject, "x", minutes).await;
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

pub fn a_click_moves_the_cursor_and_the_open_pane_follows_it() {
    crate::gtk_case(async {
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

/// Eleven steps in one turn of the loop are one body read (#1609). Counted at the storage seam,
/// which sees the store owner's thread too.
pub fn a_sweep_reads_the_row_it_lands_on_not_every_row_it_passes() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        for at in 0..12 {
            let (message, _) = fixture
                .file_from_ada(&format!("Message {at:02}"), "x", 10 + i64::from(at))
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
