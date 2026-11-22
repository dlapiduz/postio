use crate::support::{self, Fixture};

async fn budgets() -> (postio_gtk::window::FocusWindow, Fixture) {
    let fixture = Fixture::empty().await;
    fixture.file_from_ada("Q3 budget", "Numbers.", 5).await;
    fixture
        .file(
            ("Tomas Ruiz", "tomas@example.com"),
            "Travel budget",
            "Trains.",
            30,
        )
        .await;
    fixture
        .file(
            ("Grace Ho", "grace@example.com"),
            "Budget review",
            "Thursday.",
            90,
        )
        .await;
    fixture
        .file(("Lin Park", "lin@example.com"), "Lunch", "Noon?", 10)
        .await;
    let (window, _client) = fixture.open().await;
    support::rows_shown(&window, 4).await;
    (window, fixture)
}

async fn show_all(
    window: &postio_gtk::window::FocusWindow,
) -> std::rc::Rc<postio_gtk::results::ResultsPane> {
    support::press(window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    bar.set_text("budget");
    assert!(
        crate::settle_until(async || bar
            .dropdown()
            .texts()
            .iter()
            .any(|text| text.contains("Show all")))
        .await,
        "no Show all for budget: {:?}",
        bar.dropdown().texts()
    );
    bar.run_search_row();
    assert!(
        crate::settle_until(async || window.results().is_some()).await,
        "Show all did not open the results"
    );
    window.results().expect("the results are up")
}

pub fn show_all_opens_the_results_and_back_returns_to_the_inbox() {
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || {
                let mut subjects = results.subjects();
                subjects.sort();
                subjects.dedup();
                subjects == ["Budget review", "Q3 budget", "Travel budget"]
            })
            .await,
            "not the three budgets: {:?}",
            results.subjects()
        );
        assert!(
            results.count_line().starts_with('3'),
            "the count says {:?}",
            results.count_line()
        );
        assert!(
            !window.bar().is_some_and(|bar| bar.is_open()),
            "the bar stayed up"
        );
        let chrome = window.chrome().expect("the top bar");
        let said = support::texts(chrome.center());
        assert!(
            said.iter().any(|line| line == "budget")
                && said.iter().any(|line| line.contains("to edit"))
                && !said
                    .iter()
                    .any(|line| line == postio_ui::focus_row::COMMAND_PROMPT),
            "the top bar's field says {said:?}"
        );
        assert!(
            !support::texts(results.widget())
                .iter()
                .any(|line| line.contains("to edit")),
            "a second field under the top bar: {:?}",
            support::texts(results.widget())
        );

        assert!(
            crate::settle_until(async || {
                window.observe().keyboard.region == postio_ui::observe::Region::Results
            })
            .await,
            "the keyboard is in {:?}, not the results",
            window.observe().keyboard.region
        );

        let first = results.cursor();
        assert!(first.is_some(), "no ring on the results");
        support::press(&window, "j", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || results.cursor() != first).await,
            "j did not move the ring off {first:?}"
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.results().is_none()).await,
            "Escape did not leave the results"
        );
        assert_eq!(
            support::subjects(&window).len(),
            4,
            "the inbox is not whole"
        );
        assert!(
            support::texts(chrome.center())
                .iter()
                .any(|line| line == postio_ui::focus_row::COMMAND_PROMPT),
            "the command field is not back"
        );
    });
}

pub fn quick_look_stays_put_while_j_walks() {
    use gtk::prelude::*;
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Q3 budget", "Numbers.", 5).await;
        fixture
            .file(
                ("Tomas Ruiz", "tomas@example.com"),
                "Travel budget for the spring offsite, with the train times, the hotel \
                 rooms, the dinners, the shared taxis between the station and the venue \
                 on both days, and who pays for which of them",
                "Trains.",
                30,
            )
            .await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 2).await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "no ring on the results"
        );
        window.press_command(postio_core::CommandId::QuickLook);
        let panel = results.quick_look();
        let frame = || {
            panel
                .widget()
                .compute_bounds(&window)
                .filter(|bounds| bounds.height() > 10.0 && bounds.width() > 10.0)
        };
        let said = || support::texts(panel.widget());
        assert!(
            crate::settle_until(async || panel.is_open() && frame().is_some()).await,
            "Quick Look did not open"
        );
        crate::settle();
        let before = frame();
        let rows = results
            .list()
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_then(|rows| rows.compute_bounds(&window))
            .expect("the rows have a place");
        let panel_at = before.expect("the panel has a place");
        let above = panel_at.y() - rows.y();
        let below = rows.y() + rows.height() - (panel_at.y() + panel_at.height());
        assert!(
            (0.0..=40.0).contains(&above) && (0.0..=40.0).contains(&below),
            "the panel does not fill the rows' place: {above} above, {below} below"
        );
        let first = said();
        for _ in 0..3 {
            let was = said();
            window.press_command(postio_core::CommandId::NextMessage);
            assert!(
                crate::settle_until(async || said() != was).await,
                "j did not move Quick Look on from {first:?}"
            );
            crate::settle();
            assert_eq!(frame(), before, "the panel moved: {:?}", said());
        }
    });
}

pub fn the_results_open_on_their_first_heading() {
    use gtk::prelude::*;
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        for n in 0..25 {
            fixture
                .file_from_ada(&format!("Budget notes {n}"), "Who spends what.", n + 1)
                .await;
        }
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() >= 10).await,
            "the inbox never reached the screen"
        );
        let results = show_all(&window).await;
        let list = results.list().clone();
        let scroller = list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the results scroll");
        let at_the_top = || {
            results.cursor() == Some(0)
                && scroller.vadjustment().value() < 0.5
                && support::with_class(&list, "focus-results-header")
                    .into_iter()
                    .filter_map(|header| header.compute_bounds(&scroller))
                    .any(|bounds| bounds.y().abs() < 0.5)
        };
        assert!(
            crate::settle_until(async || at_the_top()).await,
            "the results did not open on their first heading: scrolled {}",
            scroller.vadjustment().value()
        );
        for _ in 0..2 {
            window.press_command(postio_core::CommandId::ToggleResultOrder);
            crate::settle();
        }
        assert!(
            crate::settle_until(async || at_the_top()).await,
            "a sort did not come back to the first heading: scrolled {}",
            scroller.vadjustment().value()
        );
    });
}

pub fn an_unread_result_has_the_inboxs_accent_dot() {
    use gtk::prelude::*;
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        let dots = || {
            support::with_class(results.list(), "focus-results-gutter")
                .into_iter()
                .filter(|gutter| {
                    gutter
                        .downcast_ref::<gtk::Label>()
                        .is_some_and(|label| label.text() == "\u{25cf}")
                })
                .collect::<Vec<_>>()
        };
        assert!(
            crate::settle_until(async || !dots().is_empty()).await,
            "no unread result drew a dot"
        );
        let manager = adw::StyleManager::default();
        let accent = manager.accent_color().to_standalone_rgba(manager.is_dark());
        // A theme may hand the accent over with a channel past the gamut's edge, which is drawn as
        // its edge.
        let channels = |colour: gtk::gdk::RGBA| {
            [colour.red(), colour.green(), colour.blue(), colour.alpha()].map(|c| c.clamp(0.0, 1.0))
        };
        for dot in dots() {
            let (drawn, wanted) = (channels(dot.color()), channels(accent));
            assert!(
                drawn.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.01),
                "the unread dot is not the accent: {drawn:?} for {wanted:?}"
            );
        }
    });
}

/// "Best match" and "Newest" are not one width, and the tabs beside Sort once slid (#2082).
pub fn the_tabs_hold_still_when_the_sort_changes() {
    use gtk::prelude::*;
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        let tabs = support::only(results.widget(), "focus-results-tabs");
        let at = || tabs.compute_bounds(&window).map(|bounds| bounds.x());
        assert!(
            crate::settle_until(async || at().is_some_and(|x| x > 0.0)).await,
            "the tabs never drew"
        );
        let before = at();
        let said = results.sort_said();
        window.press_command(postio_core::CommandId::ToggleResultOrder);
        assert!(
            crate::settle_until(async || results.sort_said() != said).await,
            "Sort did not change"
        );
        crate::settle();
        assert_eq!(at(), before, "the tabs moved with Sort's words");
    });
}

pub fn a_click_away_from_a_popover_keeps_nothing() {
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(crate::settle_until(async || results.click_filter("From")).await);
        let popover = results.popover();
        assert!(
            crate::settle_until(async || popover.is_open() && !popover.rows().is_empty()).await
        );
        popover.key(gtk::gdk::Key::space);
        popover.dismiss();
        assert!(
            crate::settle_until(async || !popover.is_open()).await,
            "the popover stayed up"
        );
        crate::settle();
        assert!(
            results.chips().is_empty(),
            "a dismissed popover kept {:?}",
            results.chips()
        );
    });
}

pub fn a_drag_across_the_timeline_narrows_the_dates() {
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        let timeline = results.timeline();
        assert!(
            crate::settle_until(async || timeline.bars().len() >= 12).await,
            "no timeline: {:?}",
            timeline.bars()
        );
        let newest = timeline.bars().len() as u32 - 1;
        timeline.drag(newest, newest);
        assert!(
            crate::settle_until(async || !results.chips().is_empty()).await,
            "the drag set no dates"
        );
        assert!(
            crate::settle_until(async || timeline.bars().last().is_some_and(|(_, on)| *on)).await,
            "the newest month is not drawn selected: {:?}",
            timeline.bars()
        );
    });
}

pub fn escape_closes_a_dropdown_over_the_results_then_leaves_them() {
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let _results = show_all(&window).await;
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        assert!(
            bar.is_open(),
            "/ did not open the dropdown over the results"
        );
        assert!(
            support::deliver(&window, "Escape"),
            "the first Escape was dropped"
        );
        assert!(!bar.is_open(), "Escape left the dropdown up");
        assert!(
            window.results().is_some(),
            "the first Escape left the results"
        );
        assert_eq!(
            window.observe().keyboard.region,
            postio_ui::observe::Region::Results,
            "the keyboard is not back on the results"
        );
        assert!(
            crate::settle_until(async || {
                let seen = window.observe().keyboard;
                seen.reachable && seen.widget.contains("GtkListView")
            })
            .await,
            "the keyboard did not come back to the results' list: {}",
            window.observe().keyboard.widget
        );
        assert!(
            support::deliver(&window, "Escape"),
            "the second Escape was dropped"
        );
        assert!(
            crate::settle_until(async || window.results().is_none()).await,
            "the second Escape did not leave the results"
        );
    });
}

/// A row drawn again is a new widget, and a keyboard on the old one went to nothing or to the next
/// control GTK found (the results' Back button, where Space presses it). The keyboard stays in the
/// list (#2136).
pub fn checking_and_walking_the_results_keeps_the_keyboard_in_their_list() {
    crate::gtk_case(async {
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        let in_the_list = || {
            let seen = window.observe().keyboard;
            seen.reachable && seen.widget.contains("GtkListView")
        };
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew"
        );
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        assert!(support::deliver(&window, "Escape"), "Escape was dropped");
        assert!(
            crate::settle_until(async || in_the_list()).await,
            "the results never had the keyboard back: {}",
            window.observe().keyboard.widget
        );
        let said = || support::texts(results.widget());
        assert!(support::deliver(&window, "x"), "x was dropped");
        assert!(
            crate::settle_until(async || said().iter().any(|text| text == "1 selected")).await,
            "x did not check the ringed result: {:?}",
            said()
        );
        assert!(
            crate::settle_while(async || in_the_list()).await,
            "x took the keyboard out of the results' list: {}",
            window.observe().keyboard.widget
        );
        let first = results.cursor();
        assert!(support::deliver(&window, "j"), "j was dropped");
        assert!(
            crate::settle_until(async || results.cursor() != first).await,
            "j did not move the ring off {first:?}"
        );
        assert!(
            crate::settle_while(async || in_the_list()).await,
            "j took the keyboard out of the results' list: {}",
            window.observe().keyboard.widget
        );
    });
}

pub fn ctrl_s_opens_the_save_popover_and_return_saves() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Q3 budget", "Numbers.", 5).await;
        fixture
            .file(
                ("Tomas Ruiz", "tomas@example.com"),
                "Travel budget",
                "Trains.",
                30,
            )
            .await;
        fixture
            .file(("Lin Park", "lin@example.com"), "Lunch", "Noon?", 10)
            .await;
        fixture.index().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "# Mine.\n").expect("a config");
        let config = postio_config::Config::load_from_path(&path).expect("it reads");
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt_at(
            &window,
            fixture.host(),
            &config,
            Some(&path),
        ));
        support::keep(directory);
        support::rows_shown(&window, 3).await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew"
        );
        support::press(&window, "s", gtk::gdk::ModifierType::CONTROL_MASK);
        let save = results.save_popover();
        assert!(
            crate::settle_until(async || save.is_open()).await,
            "Ctrl+S opened no Save popover"
        );
        assert!(!save.name().is_empty(), "no name is offered");
        assert!(
            save.texts()
                .iter()
                .any(|text| text == "Save as a saved search"),
            "{:?}",
            save.texts()
        );
        assert!(
            save.texts().iter().any(|text| text == "budget"),
            "the query's terms are not shown: {:?}",
            save.texts()
        );
        assert!(
            save.name_has_keyboard(),
            "the Name field does not have the keyboard"
        );
        assert!(
            save.key(gtk::gdk::Key::Return),
            "Return was not the popover's"
        );
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("query = \"budget\"")
            })
            .await,
            "Return saved nothing:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            crate::settle_until(async || !save.is_open()).await,
            "the popover stayed up after saving"
        );
        assert!(
            crate::settle_until(async || {
                let seen = window.observe().keyboard;
                seen.reachable
                    && seen.region == postio_ui::observe::Region::Results
                    && seen.widget.contains("GtkListView")
            })
            .await,
            "the keyboard did not come back to the results: {}",
            window.observe().keyboard.widget
        );
    });
}
