//! The command bar (US4, T086; screens 07-09): `/` opens one bar for
//! search, places and commands.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// Open the bar over `window` with `/`.
fn open_bar(window: &postio_focus::window::FocusWindow) -> std::rc::Rc<postio_focus::bar::Bar> {
    support::press(window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    assert!(bar.is_open(), "the bar is up");
    bar
}

/// Type `text` into the bar one character at a time, as a person does.
async fn type_in(bar: &postio_focus::bar::Bar, text: &str) {
    for end in 1..=text.chars().count() {
        let typed: String = text.chars().take(end).collect();
        bar.set_text(&typed);
        crate::settle();
    }
}

/// US4 scenario 2: half-typed operators are still being typed, not wrong:
/// nothing in the bar says error, and the results go on answering.
pub fn half_typed_operators_show_no_error_and_results_keep_updating() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Budget",
                "The numbers.",
                5,
            )
            .await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Version three.",
                10,
            )
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        for half in ["is:", "after:2026-", "from:"] {
            bar.set_text("");
            type_in(&bar, half).await;
            crate::settle();
            let said = bar.texts().join(" | ").to_lowercase();
            for word in ["error", "invalid", "unknown", "not a"] {
                assert!(!said.contains(word), "{half:?} shows {word:?}: {said}");
            }
        }
        bar.set_text("");
        type_in(&bar, "from:ada").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Budget"]).await,
            "from:ada did not find Ada's message: {:?}",
            bar.result_subjects()
        );
        type_in(&bar, "from:lena").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Harbor draft"]).await,
            "the results did not keep up: {:?}",
            bar.result_subjects()
        );
    });
}

/// US4 scenario 4: `in:Rec` offers Receipts, and its conversations are
/// listed newest first, with the folder's count.
pub fn in_rec_lists_receipts_newest_first() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Inbox mail", "Hello.", 5)
            .await;
        let receipts = fixture.folder("Receipts").await;
        fixture.file_in(receipts, "Train ticket", 300).await;
        fixture.file_in(receipts, "Coffee beans", 30).await;
        fixture.file_in(receipts, "Bookshop order", 90).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "in:Rec").await;
        assert!(
            crate::settle_until(async || {
                bar.heading() == "Receipts \u{b7} folder \u{b7} 3 conversations \u{b7} newest first"
            })
            .await,
            "no Receipts heading: {:?}",
            bar.heading()
        );
        assert!(
            crate::settle_until(async || {
                bar.result_subjects() == ["Coffee beans", "Bookshop order", "Train ticket"]
            })
            .await,
            "not Receipts, newest first: {:?}",
            bar.result_subjects()
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!bar.is_open(), "Escape closed the bar");
    });
}

/// US4 scenario 5: a saved search pinned at `Alt+2` runs from the list,
/// and its results are shown.
pub fn alt_2_runs_the_second_saved_search() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Budget",
                "The numbers.",
                5,
            )
            .await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Train ticket",
                "Your ticket.",
                10,
            )
            .await;
        fixture.index().await;
        let config = postio_config::Config::from_toml_str(
            "[filters.tickets]\nquery = \"subject:ticket\"\npinned = true\norder = 1\n\n\
             [filters.from-ada]\nquery = \"from:ada\"\npinned = true\norder = 2\nname = \"From Ada\"\n",
        )
        .expect("a config");
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &config,
        ));
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        support::press(&window, "2", gtk::gdk::ModifierType::ALT_MASK);
        let bar = window.bar().expect("a bar");
        assert!(bar.is_open(), "Alt+2 opened the bar");
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Budget"]).await,
            "the second saved search's results are not shown: {:?}",
            bar.result_subjects()
        );
        let said = bar.texts();
        assert!(
            said.contains(&"From Ada".to_owned()) && said.contains(&"alt+2".to_owned()),
            "the saved row names it with its key: {said:?}"
        );
        // Each pill is the control for its search: a click on the first
        // runs it, as Alt+1 does.
        let pills = support::with_class(bar.widget(), "focus-bar-saved-pill");
        let first = pills
            .first()
            .and_then(|pill| pill.downcast_ref::<gtk::Button>())
            .expect("a pill that can be pressed");
        support::click(&window, first, 1);
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Train ticket"]).await,
            "pressing the first pill did not run it: {:?}",
            bar.result_subjects()
        );
    });
}

/// US4 scenario 7: with no network, a search answers from this machine's
/// index, and within its budget -- counted, not timed. Each keystroke is at
/// most one search request to the store's owner and nothing else; what that
/// request costs at the store is the index's own budget, a flat four
/// statements a page whatever it matches (postio-index
/// `search_statement_budget.rs`).
pub fn offline_search_answers_locally_one_request_a_keystroke() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Budget",
                "The numbers.",
                5,
            )
            .await;
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
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session =
            postio_focus::startup::adopt(&window, host, &postio_config::Config::default());
        let client = session.client().clone();
        support::keep(session);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        assert!(sink.emit(postio_core::Event::ConnectionChanged {
            account: fixture.account.id,
            state: postio_core::ConnectionState::Offline,
        }));
        assert!(crate::settle_until(async || window.sync_said() == "Offline").await);

        let bar = open_bar(&window);
        // Opening reads where the bar can go; typing is what is counted.
        assert!(crate::settle_until(async || bar.places_known()).await);
        let before = client.counts().snapshot();
        type_in(&bar, "from:ada").await;
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
        // "f" to "from" are plain words, which search nothing until asked;
        // "from:" to "from:ada" name an operator, and each is one search.
        assert_eq!(
            asked,
            [("SearchHits", 4)],
            "one search a keystroke that names an operator, and nothing else asked for"
        );
    });
}

/// Screen 09: a plain word is answered with the commands and places it
/// names and one search row, not a search; the search runs when that row
/// is chosen. Words that lower to operators (screen 07) search at once.
pub fn a_plain_word_offers_commands_and_searches_only_when_asked() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Archive plan",
                "Boxes.",
                5,
            )
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "arch").await;
        // Long enough for a search to have answered, had one been asked.
        crate::settle_for(std::time::Duration::from_millis(500)).await;
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line == "Archive"),
            "the command: {said:?}"
        );
        assert!(
            said.iter().any(|line| line.starts_with("Search mail for")),
            "the search row: {said:?}"
        );
        assert!(bar.result_subjects().is_empty(), "no search until asked");
        // A plain word is what the entry already shows; it is not a chip.
        assert!(
            bar.chips().is_empty(),
            "a plain word chipped: {:?}",
            bar.chips()
        );
        bar.run_search();
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Archive plan"]).await,
            "choosing the search row did not search: {:?}",
            bar.result_subjects()
        );
    });
}

/// US4 scenario 1 (screen 07): "the invoice Ada sent last month" is read as
/// words, a correspondent and a month -- `from:ada` because the address
/// book knows an Ada -- and finds only Ada's invoice from last month.
pub fn a_sentence_names_its_sender_from_the_address_book() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        use chrono::Datelike as _;
        let fixture = Fixture::empty().await;
        // The bar reads "last month" against today's calendar, so the mail
        // is dated in it: the tenth of last month, and today.
        let today = chrono::Local::now().date_naive();
        let last_month = today
            .with_day(1)
            .and_then(|first| first.checked_sub_months(chrono::Months::new(1)))
            .and_then(|first| first.with_day(10))
            .expect("a tenth of last month");
        let then = last_month.and_hms_opt(12, 0, 0).expect("noon").and_utc();
        let ago = (support::now() - then).num_minutes();
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Invoice 2026-08",
                "Attached.",
                ago,
            )
            .await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Invoice for the venue",
                "Attached.",
                ago,
            )
            .await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Invoice, this month's",
                "Attached.",
                (support::now() - chrono::Local::now().to_utc()).num_minutes(),
            )
            .await;
        fixture.correspondent("Ada Moreno", "ada@example.com").await;
        fixture.correspondent("Lena Park", "lena@example.org").await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        assert!(crate::settle_until(async || bar.places_known()).await);
        bar.set_text("the invoice Ada sent last month");
        assert!(
            crate::settle_until(async || bar.chips().iter().any(|chip| chip == "from:ada")).await,
            "Ada was not read as a sender: {:?}",
            bar.chips()
        );
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Invoice 2026-08"]).await,
            "not only Ada's invoice from last month: {:?}",
            bar.result_subjects()
        );
    });
}

/// T086, US4: `Tab` steps from the words into the chips they were lowered
/// to, and on from chip to chip, the echo naming the one being edited;
/// `Ctrl+Backspace` goes back to the plain words; `Ctrl+S` saves the query
/// to `config.toml`, pinned, and the saved row shows it.
pub fn tab_steps_into_the_chips_and_ctrl_s_saves_the_query() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Invoice 2026-08",
                "Attached.",
                5,
            )
            .await;
        fixture.correspondent("Ada Moreno", "ada@example.com").await;
        fixture.index().await;
        let directory = tempfile::tempdir().expect("a config directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "# Mine.\n").expect("a config");
        let config = postio_config::Config::load_from_path(&path).expect("it reads");
        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_focus::startup::adopt_at(
            &window,
            fixture.host(),
            &config,
            Some(&path),
        ));
        support::keep(directory);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        assert!(crate::settle_until(async || bar.places_known()).await);
        let words = "invoices from Ada";
        bar.set_text(words);
        assert!(
            crate::settle_until(async || bar.chips() == ["invoices", "from:ada"]).await,
            "the words were not lowered: {:?}",
            bar.chips()
        );

        assert!(
            support::deliver(&window, "Tab"),
            "Tab reached the bar: {}",
            support::focus_path(&window)
        );
        assert_eq!(
            bar.typed(),
            "invoices from:ada",
            "the entry holds the chips"
        );
        assert!(
            bar.texts()
                .iter()
                .any(|text| text.contains("editing invoices")),
            "the echo names the chip: {:?}",
            bar.texts()
        );
        assert!(
            support::deliver(&window, "Tab"),
            "Tab reached the bar: {}",
            support::focus_path(&window)
        );
        assert!(
            bar.texts()
                .iter()
                .any(|text| text.contains("editing from:")),
            "Tab moved on to the next chip: {:?}",
            bar.texts()
        );

        support::press(&window, "BackSpace", gtk::gdk::ModifierType::CONTROL_MASK);
        assert_eq!(bar.typed(), words, "Ctrl+Backspace went back to the words");

        support::press(&window, "s", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("query = \"invoices from:ada\"")
            })
            .await,
            "Ctrl+S saved no query:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("# Mine."),
            "the rest of the file is kept"
        );
        assert!(
            crate::settle_until(async || bar.texts().iter().any(|text| text == "alt+1")).await,
            "the saved row does not show it: {:?}",
            bar.texts()
        );
    });
}

async fn three_and_open() -> postio_focus::window::FocusWindow {
    let fixture = Fixture::empty().await;
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Archive plan",
            "Boxes.",
            5,
        )
        .await;
    fixture.index().await;
    let (window, _client) = fixture.open().await;
    assert!(
        crate::settle_until(async || support::subjects(&window).len() == 1).await,
        "the inbox never reached the screen"
    );
    support::keep(fixture);
    window
}

/// C24 (T182): `Ctrl K` opens the bar in command mode -- `>` already filled
/// in, so only commands show -- and `/` opens it for mail search.
pub fn ctrl_k_opens_the_bar_for_commands_and_slash_for_search() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
        // What is typed after the prefix narrows the commands.
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
        type_in(&bar, "arch").await;
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line.starts_with("Search mail for")),
            "the search row: {said:?}"
        );
    });
}

/// C24 (T182): the bar opens in place -- the top bar's own field is where
/// the typing happens, and the results drop below it -- not as a panel
/// somewhere else over the list.
pub fn the_bar_opens_in_the_top_bars_field() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = three_and_open().await;
        let field = support::only(&window, "focus-command-field");
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
            !field.is_mapped() || field.opacity() < 0.1,
            "the field's own prompt still shows under the bar's input"
        );
    });
}

/// T241, ADR 0037: a word that found nothing lists the mail for the word
/// that was meant, and the bar says so, with a row that searches for the
/// typed word exactly. Ported from the classic app's `search_instead`.
pub fn a_misspelled_word_says_what_it_found_and_offers_the_typed_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Quarterly planning",
                "Numbers.",
                5,
            )
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "qarterly").await;
        bar.run_search();
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Quarterly planning"]).await,
            "the word that was meant was not searched: {:?}",
            bar.result_subjects()
        );
        let said = bar.texts();
        assert!(
            said.iter()
                .any(|line| line == "Showing results for quarterly"),
            "the bar did not say which word it searched: {said:?}"
        );
        let offer = "Search instead for \u{201c}qarterly\u{201d}";
        assert!(
            said.iter().any(|line| line == offer),
            "no row offers the typed word: {said:?}"
        );
        support::click_row_saying(&window, bar.widget(), offer);
        assert_eq!(
            bar.typed(),
            "\"qarterly\"",
            "the typed word is asked for exactly: quoted, which is never rewritten"
        );
        assert!(
            crate::settle_until(async || {
                let said = bar.texts();
                bar.result_subjects().is_empty()
                    && !said
                        .iter()
                        .any(|line| line.starts_with("Showing results for"))
            })
            .await,
            "the exact word still lists the rewritten mail: {:?}",
            bar.texts()
        );
    });
}

/// T241: `O` switches results between relevance and date, once a result is
/// under the arrows; before that it is a capital O being typed, and a row
/// says the same to a mouse.
pub fn o_reorders_the_results_once_a_row_is_chosen() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        // The ranker's two properties (postio-ffi `disagreeing`): twenty
        // messages that do not match, and hours rather than days between
        // the two that do.
        for at in 0..20 {
            fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    &format!("Entirely unrelated {at}"),
                    "Nothing here.",
                    2000 + at,
                )
                .await;
        }
        let (dense, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Report", "Report.", 600)
            .await;
        fixture
            .write_body(dense, "report report report report report")
            .await;
        let (glancing, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "One report",
                "One report among other things.",
                300,
            )
            .await;
        fixture
            .write_body(glancing, "One report among other things entirely")
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        let _ = crate::settle_until(async || support::subjects(&window).len() >= 20).await;
        let bar = open_bar(&window);
        type_in(&bar, "report").await;
        bar.run_search();
        let relevance = vec!["Report".to_owned(), "One report".to_owned()];
        let by_date = vec!["One report".to_owned(), "Report".to_owned()];
        assert!(
            crate::settle_until(async || bar.result_subjects().len() == 2).await,
            "no results: {:?}",
            bar.result_subjects()
        );
        let first = bar.result_subjects();
        assert_eq!(first, relevance, "relevance puts the denser match first");
        // Typing wins: `O` before any row is chosen is a letter, left for
        // the entry to take, and it reorders nothing.
        assert!(
            !support::deliver(&window, "O"),
            "O was claimed while nothing was chosen: it is a letter then"
        );
        assert_eq!(bar.result_subjects(), first, "a typed O reordered the rows");
        let before = bar.result_subjects();
        support::press(&window, "Down", gtk::gdk::ModifierType::empty());
        assert!(
            support::deliver(&window, "O"),
            "O was not taken with a result chosen"
        );
        assert!(
            crate::settle_until(async || {
                let now = bar.result_subjects();
                now.len() == 2 && now != before
            })
            .await,
            "O did not reorder the rows: {:?}",
            bar.result_subjects()
        );
        assert_eq!(bar.typed(), "report", "O was typed as well");
        assert_eq!(bar.result_subjects(), by_date, "newest first");
        assert!(
            bar.texts().iter().any(|line| line == "Sorted by date"),
            "the bar does not say its order: {:?}",
            bar.texts()
        );
        // The mouse has the same switch: the row that says the order.
        support::click_row_saying(&window, bar.widget(), "Sorted by date");
        assert!(
            crate::settle_until(async || bar.result_subjects() == relevance).await,
            "the order row did not switch back: {:?}",
            bar.result_subjects()
        );
    });
}
