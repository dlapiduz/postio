//! The search results view (spec 010 T171, screens 06 and 07): GTK draws
//! the controller's results in the inbox's place.
//!
//! What matches, how it is grouped and ordered, and where the ring goes
//! are the controller's, asserted in `postio-focus`. These cases drive the
//! window as a person does, with spec 010's search staged on: Show all
//! opens the results, the keys walk them, a filter button edits the query,
//! and Back brings the inbox back.

use crate::support::{self, Fixture};

/// A window over three budget messages and one other, with the search
/// staged on, its inbox drawn.
async fn budgets() -> (postio_gtk::window::FocusWindow, Fixture) {
    postio_gtk::list::feed::stage_results_view(true);
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
    postio_gtk::list::feed::stage_results_view(false);
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
