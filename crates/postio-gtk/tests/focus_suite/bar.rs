use gtk::prelude::*;

use crate::support::{self, Fixture};

fn open_bar(window: &postio_gtk::window::FocusWindow) -> std::rc::Rc<postio_gtk::bar::Bar> {
    support::press(window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    assert!(bar.is_open(), "the bar is up");
    bar
}

async fn type_in(bar: &postio_gtk::bar::Bar, text: &str) {
    for end in 1..=text.chars().count() {
        let typed: String = text.chars().take(end).collect();
        bar.set_text(&typed);
        crate::settle();
    }
}

/// Counted, not timed: each keystroke is at most one search request; the index's own budget is four
/// statements a page (postio-index `search_statement_budget.rs`).
pub fn offline_search_answers_locally_one_request_a_keystroke() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "The numbers.", 5).await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Version three.",
                10,
            )
            .await;
        fixture.index().await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
        let client = session.client().clone();
        support::keep(session);
        support::rows_shown(&window, 2).await;
        assert!(sink.emit(postio_core::Event::ConnectionChanged {
            account: fixture.account.id,
            state: postio_core::ConnectionState::Offline,
        }));
        assert!(crate::settle_until(async || window.sync_said() == "Offline").await);

        let bar = open_bar(&window);
        assert!(crate::settle_until(async || bar.places_known()).await);
        let before = client.counts().snapshot();
        type_in(&bar, "budget").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Budget"]).await,
            "offline, the local index did not answer: {:?}",
            bar.result_subjects()
        );
        let after = client.counts().snapshot();
        let asked: Vec<(&str, u64)> = after
            .iter()
            .map(|(family, count)| (*family, count - before.get(family).copied().unwrap_or(0)))
            .filter(|(_, count)| *count > 0)
            .collect();
        // Not every keystroke makes a search: a single letter is in most of a mailbox
        // (`ParsedQuery::is_searchable`). One to three letters also ask what they could become, and
        // hits ask for their passages.
        let count = |family: &str| {
            asked
                .iter()
                .find(|(asked, _)| *asked == family)
                .map_or(0, |(_, count)| *count)
        };
        assert_eq!(count("Conversations"), 5, "{asked:?}");
        assert_eq!(count("Suggest"), 3, "{asked:?}");
        assert!((1..=5).contains(&count("Passages")), "{asked:?}");
        assert!(
            asked
                .iter()
                .all(|(family, _)| ["Conversations", "Suggest", "Passages"].contains(family)),
            "something else was asked: {asked:?}"
        );
    });
}

pub fn the_fields_operator_terms_are_chips() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Archive plan", "Boxes.", 5).await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 1).await;
        let bar = open_bar(&window);
        let typed = "archive from:ada@example.com -to:ben@example.net";
        type_in(&bar, typed).await;
        let from = typed.find("from:").unwrap() as u32;
        let to = typed.find("-to:").unwrap() as u32;
        assert!(
            crate::settle_until(async || {
                bar.chip_ranges()
                    == [
                        (from, from + "from:ada@example.com".len() as u32, false),
                        (to, typed.len() as u32, true),
                    ]
            })
            .await,
            "the terms are not chips: {:?}",
            bar.chip_ranges()
        );
    });
}

async fn three_and_open() -> postio_gtk::window::FocusWindow {
    let fixture = Fixture::empty().await;
    fixture.file_from_ada("Archive plan", "Boxes.", 5).await;
    fixture.index().await;
    let (window, _client) = fixture.open().await;
    support::rows_shown(&window, 1).await;
    support::keep(fixture);
    window
}

pub fn ctrl_k_opens_the_bar_for_commands_and_slash_for_search() {
    crate::gtk_case(async {
        let window = three_and_open().await;

        support::press(&window, "k", gtk::gdk::ModifierType::CONTROL_MASK);
        let bar = window.bar().expect("Ctrl K opened the bar");
        assert!(bar.is_open(), "Ctrl K opened no bar");
        assert_eq!(bar.typed(), ">", "Ctrl K fills in the command prefix");
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line == "Commands"),
            "the commands: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line.starts_with("Search mail for")),
            "a search row in command mode: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line == "Go to"),
            "places in command mode: {said:?}"
        );
        type_in(&bar, ">arch").await;
        assert!(
            bar.texts().iter().any(|line| line == "Archive"),
            "the narrowed command: {:?}",
            bar.texts()
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!bar.is_open(), "Escape did not close the bar");

        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        assert!(bar.is_open(), "/ opened no bar");
        assert_eq!(bar.typed(), "", "/ opens the bar for search, not commands");
        type_in(&bar, "archive").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Archive plan"]).await,
            "/ did not search: {:?}",
            bar.texts()
        );
        assert!(
            !bar.texts().iter().any(|line| line == "Commands"),
            "commands under a search: {:?}",
            bar.texts()
        );
    });
}

pub fn the_bar_opens_in_the_top_bars_field() {
    crate::gtk_case(async {
        let window = three_and_open().await;
        let field = support::only(&window, "focus-command-field");
        let center = window.chrome().expect("the top bar").center().clone();
        let before = field
            .compute_bounds(&window)
            .expect("the field has a place in the window");

        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("the bar");
        type_in(&bar, "arch").await;
        let entry = bar.input();
        assert!(
            crate::settle_until(async || entry.is_mapped() && entry.width() > 0).await,
            "the bar's input was never laid out"
        );
        crate::settle();
        let at = entry
            .compute_bounds(&window)
            .expect("the input has a place in the window");
        let (x, y) = (at.x() + at.width() / 2.0, at.y() + at.height() / 2.0);
        assert!(
            x >= before.x()
                && x <= before.x() + before.width()
                && y >= before.y()
                && y <= before.y() + before.height(),
            "the bar's input is at ({x}, {y}), not in the top bar's field {before:?}"
        );
        let results = support::only(&window, "focus-bar-results")
            .compute_bounds(&window)
            .expect("the results have a place in the window");
        assert!(
            results.y() >= before.y() + before.height() - 1.0,
            "the results are not below the field: {results:?} under {before:?}"
        );
        assert!(
            !field.is_mapped() || center.opacity() < 0.1,
            "the field's own prompt still shows under the bar's input"
        );
    });
}

pub fn a_result_gives_its_subject_room_before_its_first_line() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let subject = "[harbour-dev] Tide gate interlock";
        fixture
            .file_from_ada(subject,
                "The harbour proposal below changes how the tide gate interlock is armed during maintenance windows and why.",
                5,
            )
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open_sized(Some((1280, 800))).await;
        support::rows_shown(&window, 1).await;
        let bar = open_bar(&window);
        type_in(&bar, "harbour").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == [subject]).await,
            "typing searched nothing: {:?}",
            bar.result_subjects()
        );
        let label = || {
            support::with_class(bar.dropdown().widget(), "focus-dropdown-row-title")
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .find(|label| label.text().contains(subject) && label.width() > 0)
        };
        assert!(
            crate::settle_until(async || label().is_some()).await,
            "the hit was never laid out"
        );
        let label = label().expect("laid out");
        assert!(
            !label.layout().is_ellipsized(),
            "the subject was cut ({} px) while the first line kept its room",
            label.width()
        );
    });
}

pub fn a_hit_steps_the_results_and_its_own_thread() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .file_from_ada("Harbor budget", "Numbers for harbor.", 5)
            .await;
        fixture.thread_of("Harbor draft", 3, 20).await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        support::rows_shown(&window, 2).await;
        let bar = open_bar(&window);
        type_in(&bar, "harbor").await;
        assert!(
            crate::settle_until(async || bar.result_subjects().len() >= 2).await,
            "the search found {:?}",
            bar.result_subjects()
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.results().is_some()).await,
            "Show all did not open the results"
        );
        let results = window.results().expect("the results");
        assert!(
            crate::settle_until(async || results.subjects().len() == 4).await,
            "the results found {:?}",
            results.subjects()
        );
        let mut hits = results.subjects();
        let mut seen = std::collections::HashSet::new();
        hits.retain(|subject| seen.insert(subject.clone()));
        chain(&window, "Return");
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
            "the result never opened"
        );
        let reading = window.reading().expect("the hit opened");
        assert!(
            crate::settle_until(async || reading.is_open() && reading.title() == hits[0]).await,
            "the first hit did not open: {:?} of {hits:?}",
            reading.title()
        );
        assert!(
            reading
                .subtitle()
                .starts_with(&format!("Result 1 of {}", hits.len())),
            "{:?}",
            reading.subtitle()
        );

        assert!(
            crate::settle_until(async || reading.reader().view().is_mapped()).await,
            "the dialog never drew"
        );
        chain(&window, "j");
        assert!(
            crate::settle_until(async || reading.title() == hits[1]).await,
            "j did not step to the next result: {:?}",
            reading.title()
        );
        assert!(
            reading
                .subtitle()
                .starts_with(&format!("Result 2 of {}", hits.len())),
            "{:?}",
            reading.subtitle()
        );
        chain(&window, "k");
        assert!(
            crate::settle_until(async || reading.title() == hits[0]).await,
            "k did not step back: {:?}",
            reading.title()
        );

        let thread_hit = hits
            .iter()
            .position(|subject| subject.ends_with("Harbor draft"))
            .expect("the thread is among the hits");
        while reading.title() != hits[thread_hit] {
            chain(&window, "j");
            crate::settle();
        }
        assert!(
            crate::settle_until(async || reading.thread_known() == 3).await,
            "the hit's thread was never read: {}",
            reading.thread_known()
        );
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message ")).await,
            "the hit's body never drew: {:?}",
            reading.body_text()
        );
        let toward_middle = if reading.body_text().contains("Message 1") {
            "n"
        } else {
            "p"
        };
        chain(&window, toward_middle);
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 2")).await,
            "{toward_middle} did not step the hit's thread: {:?}",
            reading.body_text()
        );
        assert!(
            reading.subtitle().contains("Message 2 of 3"),
            "{:?}",
            reading.subtitle()
        );

        chain(&window, "Escape");
        assert!(
            crate::settle_until(async || {
                window.reading().is_none_or(|reading| !reading.is_open())
                    && window.results().is_some()
            })
            .await,
            "the results did not come back"
        );
    });
}

fn chain(window: &postio_gtk::window::FocusWindow, key: &str) {
    let chord: postio_ui::keymap::Chord = key.parse().expect("a chord");
    let delivery = postio_widgets::storyboard::deliver::press(window.upcast_ref(), &chord)
        .expect("a deliverable chord");
    assert!(
        matches!(
            delivery,
            postio_widgets::storyboard::deliver::Delivery::Delivered { .. }
        ),
        "{key} was dropped"
    );
    crate::settle();
}

pub fn walking_the_results_keeps_the_ringed_row_in_view() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        for n in 0..25 {
            fixture
                .file_from_ada(&format!("Standup notes {n}"), "Who is doing what.", n + 1)
                .await;
        }
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        let bar = open_bar(&window);
        type_in(&bar, "standup").await;
        assert!(
            crate::settle_until(async || !bar.result_subjects().is_empty()).await,
            "standup found nothing: {:?}",
            bar.texts()
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.results().is_some()).await,
            "Show all did not open the results"
        );
        let results = window.results().expect("the results");
        let list = results.list().clone();
        let scroller = list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the results scroll");
        assert!(
            crate::settle_until(async || {
                scroller.height() > 0
                    && scroller.vadjustment().upper() > f64::from(scroller.height()) + 100.0
            })
            .await,
            "the results must overflow for this to mean anything: {} in a scroller of {}",
            scroller.vadjustment().upper(),
            scroller.height()
        );
        let in_view = || {
            let Some(ringed) = support::with_class(&list, "focus-results-cursor")
                .into_iter()
                .next()
            else {
                return false;
            };
            let Some(bounds) = ringed.compute_bounds(&scroller) else {
                return false;
            };
            bounds.y() >= -0.5 && bounds.y() + bounds.height() <= scroller.height() as f32 + 0.5
        };
        for step in 1..=24 {
            chain(&window, "j");
            assert!(
                crate::settle_until(async || in_view()).await,
                "after {step} presses of j the ringed row is out of view: {:?} of {} rows, ringed at {:?} in a scroller {} high at {}",
                results.cursor(),
                results.subjects().len(),
                support::with_class(&list, "focus-results-cursor")
                    .iter()
                    .map(|ringed| ringed.compute_bounds(&scroller))
                    .collect::<Vec<_>>(),
                scroller.height(),
                scroller.vadjustment().value()
            );
        }
        for step in 1..=24 {
            chain(&window, "k");
            assert!(
                crate::settle_until(async || in_view()).await,
                "after {step} presses of k the ringed row is out of view"
            );
        }
    });
}
