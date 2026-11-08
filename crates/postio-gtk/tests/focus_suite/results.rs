//! The search results view (spec 010 T171, screens 06 and 07): GTK draws
//! the controller's results in the inbox's place.
//!
//! What matches, how it is grouped and ordered, and where the ring goes
//! are the controller's, asserted in `postio-focus`. These cases drive the
//! window as a person does: Show all
//! opens the results, the keys walk them, a filter button edits the query,
//! and Back brings the inbox back.

use crate::support::{self, Fixture};

/// A window over three budget messages and one other, its inbox drawn.
async fn budgets() -> (postio_gtk::window::FocusWindow, Fixture) {
    let fixture = Fixture::empty().await;
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Q3 budget",
            "Numbers.",
            5,
        )
        .await;
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
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 4).await,
        "the inbox never reached the screen"
    );
    (window, fixture)
}

/// `/`, "budget", and Return on Show all: the results, open.
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

/// Show all puts the three matches in the inbox's place, with their count;
/// `j` moves the ring to the next; Escape gives the inbox back, all four
/// rows, with the keyboard.
pub fn show_all_opens_the_results_and_back_returns_to_the_inbox() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || {
                // Top hits draws them again above their months.
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

        // A storyboard says where the keyboard is as a region: the
        // results', which the Mac's storyboards check for (#1825).
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
    });
}

/// A toggle filter button adds its term as a chip, and the chip's ✕ takes
/// it out again.
pub fn a_toggle_filter_adds_its_chip_and_the_chip_removes_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || !results.filters().is_empty()).await,
            "no filter buttons"
        );
        let toggle = results
            .filters()
            .into_iter()
            .map(|(label, _)| label)
            .find(|label| !label.ends_with('\u{25be}'))
            .expect("a toggle button");
        assert!(results.click_filter(&toggle));
        assert!(
            crate::settle_until(async || results.chips().len() == 1).await,
            "{toggle} added no chip: {:?}",
            results.chips()
        );
        assert!(
            results
                .filters()
                .iter()
                .any(|(label, applied)| *label == toggle && *applied),
            "{toggle} is not drawn applied: {:?}",
            results.filters()
        );
        assert!(results.remove_chip(0), "no chip to remove");
        assert!(
            crate::settle_until(async || results.chips().is_empty()).await,
            "the chip stayed: {:?}",
            results.chips()
        );
    });
}

/// From ▾ opens its popover over the people in the results; Space checks
/// the one the arrows rest on and Return keeps it, as a `from:` chip.
pub fn from_checks_a_person_and_return_keeps_the_chip() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.click_filter("From")).await,
            "no From button"
        );
        let popover = results.popover();
        assert!(
            crate::settle_until(async || popover.is_open() && popover.rows().len() == 3).await,
            "From did not list the three senders: {:?}",
            popover.rows()
        );
        assert!(
            popover.key(gtk::gdk::Key::space),
            "Space was not the popover's"
        );
        assert!(
            crate::settle_until(async || popover.rows().iter().filter(|(_, on)| *on).count() == 1)
                .await,
            "Space checked nothing: {:?}",
            popover.rows()
        );
        assert!(popover.key(gtk::gdk::Key::Return));
        assert!(
            crate::settle_until(async || !popover.is_open()
                && results.chips().iter().any(|chip| chip.starts_with("from:")))
            .await,
            "Return kept no from: chip: {:?}",
            results.chips()
        );
    });
}

/// A click away from a popover puts the query back as it was: what it
/// previewed is not kept.
pub fn a_click_away_from_a_popover_keeps_nothing() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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

/// The timeline draws a bar a month; a drag across the newest narrows the
/// query to it, and the bar is drawn selected.
pub fn a_drag_across_the_timeline_narrows_the_dates() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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

/// The People tab lists who the results are from and to; Return on one
/// searches their mail: a `from:` chip, and the Conversations tab again.
pub fn return_on_a_person_searches_from_them() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.click_tab("People")).await,
            "no People tab"
        );
        assert!(
            crate::settle_until(async || {
                let mut names = results.people().names();
                names.sort();
                names == ["Ada Moreno", "Grace Ho", "Tomas Ruiz"]
            })
            .await,
            "not the three senders: {:?}",
            results.people().names()
        );
        window.press_command(postio_core::CommandId::OpenMessage);
        assert!(
            crate::settle_until(async || results
                .chips()
                .iter()
                .any(|chip| chip.starts_with("from:")))
            .await,
            "Return searched nobody: {:?}",
            results.chips()
        );
    });
}

/// A filter that leaves nothing puts the no-results page in the rows'
/// place, with a looser search that finds something; running it takes
/// the filter off and brings the results back.
pub fn nothing_found_offers_a_looser_search_that_finds_some() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.click_filter("Attachment")).await,
            "no Attachment button"
        );
        assert!(
            crate::settle_until(
                async || results.showing_nothing() && !results.nothing().ways().is_empty()
            )
            .await,
            "no page offering a way out: {:?}",
            results.nothing().ways()
        );
        assert!(
            !results.nothing().title().is_empty(),
            "the page says nothing"
        );
        window.press_command(postio_core::CommandId::PickRelaxation1);
        assert!(
            crate::settle_until(async || !results.showing_nothing() && results.chips().is_empty())
                .await,
            "the looser search did not bring the results back: {:?}",
            results.chips()
        );
        assert!(
            crate::settle_until(async || results.subjects().len() >= 3).await,
            "the budgets are not back: {:?}",
            results.subjects()
        );
    });
}

/// Spec 010 R10 (#1825): Ctrl+1-3 pick the results' tabs from the keyboard,
/// and the keys go on reaching the results on each.
pub fn the_tabs_go_by_their_keys() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew"
        );
        let ctrl = gtk::gdk::ModifierType::CONTROL_MASK;
        // Back to the inbox and into the results again first: the second
        // time is where the keyboard was left on a page no longer shown.
        assert!(
            support::deliver_with(&window, "bracketleft", ctrl),
            "History back was dropped"
        );
        assert!(
            crate::settle_until(async || window.results().is_none()).await,
            "History back did not go back to the inbox"
        );
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew again"
        );
        assert!(
            support::deliver_with(&window, "2", ctrl),
            "Ctrl+2 was dropped"
        );
        assert!(
            crate::settle_until(async || results.showing_files()).await,
            "Ctrl+2 did not show the Files tab"
        );
        assert!(
            support::deliver_with(&window, "3", ctrl),
            "Ctrl+3 was dropped"
        );
        assert!(
            crate::settle_until(async || results.showing_people()).await,
            "Ctrl+3 did not show the People tab"
        );
        assert!(
            support::deliver_with(&window, "1", ctrl),
            "Ctrl+1 was dropped"
        );
        assert!(
            crate::settle_until(async || !results.showing_people() && !results.showing_files())
                .await,
            "Ctrl+1 did not go back to the conversations"
        );
        assert!(
            support::deliver(&window, "j"),
            "the conversations' keys were dropped after the tabs"
        );
    });
}

/// Spec 010 (results-esc-ladder, #1825): `/` over the results opens the
/// dropdown; Escape closes it and the results have the keyboard again;
/// the next Escape leaves them for the inbox.
pub fn escape_closes_a_dropdown_over_the_results_then_leaves_them() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
            crate::settle_until(async || window.observe().keyboard.reachable).await,
            "the keyboard went to the inbox behind the results: {}",
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

/// #1825: a search that found nothing, left, and then one that finds
/// something: its rows are shown, not the first one's "Nothing matches".
pub fn a_search_after_one_that_found_nothing_shows_its_rows() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        bar.set_text("zebra");
        crate::settle();
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.results().is_some_and(|r| r.showing_nothing()))
                .await,
            "zebra did not say it found nothing"
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.results().is_none()).await,
            "Escape did not leave the results"
        );
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || !results.subjects().is_empty()).await,
            "budget found nothing"
        );
        assert!(
            !results.showing_nothing(),
            "the last search's \"Nothing matches\" is shown over budget's rows"
        );
    });
}

/// storyboards/flows/search-open-reply-back on spec 010 (#1825): a result
/// opened from the results answers `r` with a reply quoting it, and Escape
/// walks back out: the reply, the message, then the results.
pub fn replying_to_a_result_and_escaping_back_to_the_results() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew"
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
            "Return did not open the result"
        );
        support::keys(&window, &["r"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "r opened no reply"
        );
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Escape left the reply up"
        );
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.reading().is_none_or(|r| !r.is_open())).await,
            "the second Escape left the message open"
        );
        assert!(window.results().is_some(), "the results are gone");
        assert!(
            crate::settle_until(async || {
                let seen = window.observe().keyboard;
                seen.reachable && seen.region == postio_ui::observe::Region::Results
            })
            .await,
            "the keyboard is not back on the results: {}",
            window.observe().keyboard.widget
        );
        // On the ringed result itself, not on a group's heading: one ring.
        let on_the_ring = || {
            use gtk::prelude::WidgetExt;
            gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus| {
                std::iter::successors(Some(focus.clone()), |widget| widget.first_child())
                    .any(|widget| widget.has_css_class("focus-results-cursor"))
            })
        };
        assert!(
            crate::settle_until(async || on_the_ring()).await,
            "the keyboard is not on the ringed result: {}",
            support::focus_path(&window)
        );
    });
}

/// Spec 010 US5 (§3.5): checking results turns the footer into the bulk
/// bar, which says how many the next verb will reach.
pub fn checking_results_says_how_many_are_selected() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.cursor().is_some()).await,
            "the results never drew"
        );
        let said = || support::texts(results.widget());
        support::keys(&window, &["x"]);
        assert!(
            crate::settle_until(async || said().iter().any(|text| text == "1 selected")).await,
            "the bulk bar does not count one: {:?}",
            said()
        );
        support::keys(&window, &["j", "x"]);
        assert!(
            crate::settle_until(async || said().iter().any(|text| text == "2 selected")).await,
            "the bulk bar does not count two: {:?}",
            said()
        );
    });
}

/// Spec 010 D24 (#1825): Ctrl+Backspace on the no-results page clears the
/// filters, and the keyboard is on the results that come back -- not left
/// on the page that went, nor handed to the top bar.
pub fn clearing_the_filters_keeps_the_keyboard_on_the_results() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _fixture) = budgets().await;
        let results = show_all(&window).await;
        assert!(
            crate::settle_until(async || results.click_filter("Attachment")).await,
            "no Attachment button"
        );
        assert!(
            crate::settle_until(async || results.showing_nothing()).await,
            "nothing matched, and the page did not say so"
        );
        results.focus();
        support::press(&window, "BackSpace", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !results.showing_nothing() && results.chips().is_empty())
                .await,
            "Ctrl+Backspace did not clear the filters: {:?}",
            results.chips()
        );
        assert!(
            crate::settle_until(async || {
                let seen = window.observe().keyboard;
                seen.reachable && seen.region == postio_ui::observe::Region::Results
            })
            .await,
            "the keyboard is not on the results: {}",
            support::focus_path(&window)
        );
    });
}

/// Spec 010 US5 (§3.9, screen 12): Ctrl+S over the results opens the Save
/// popover -- a name made from the query, its terms, Pin on -- with the
/// keyboard in its Name field; Return saves it to `config.toml`, pinned,
/// and the popover goes.
pub fn ctrl_s_opens_the_save_popover_and_return_saves() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Q3 budget",
                "Numbers.",
                5,
            )
            .await;
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
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
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
