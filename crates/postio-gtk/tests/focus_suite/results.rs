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
