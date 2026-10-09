//! The command bar (US4, T086; screens 07-09): `/` opens one bar for
//! search, places and commands.

use gtk::prelude::*;

use crate::support::{self, Fixture};

/// Open the bar over `window` with `/`.
fn open_bar(window: &postio_gtk::window::FocusWindow) -> std::rc::Rc<postio_gtk::bar::Bar> {
    support::press(window, "slash", gtk::gdk::ModifierType::empty());
    let bar = window.bar().expect("/ opened the bar");
    assert!(bar.is_open(), "the bar is up");
    bar
}

/// Type `text` into the bar one character at a time, as a person does.
async fn type_in(bar: &postio_gtk::bar::Bar, text: &str) {
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
            "[saved_searches.tickets]\nquery = \"subject:ticket\"\npinned = true\norder = 1\n\n\
             [saved_searches.from-ada]\nquery = \"from:ada\"\npinned = true\norder = 2\nname = \"From Ada\"\n",
        )
        .expect("a config");
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        support::keep(postio_gtk::startup::adopt(&window, fixture.host(), &config));
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
        let window = postio_gtk::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session = postio_gtk::startup::adopt(&window, host, &postio_config::Config::default());
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
        // Every keystroke is one search, as it is typed, and nothing else.
        assert_eq!(
            asked,
            [("SearchHits", 8)],
            "one search a keystroke, and nothing else asked for"
        );
    });
}

/// Screen 09, 07 (d): a plain word is answered with the commands and
/// places it names, the one search row, and the results of the search as it
/// is typed -- under the search row, which stays. Return on the row takes
/// the keyboard to the first hit. Each hit shows the message's first line.
pub fn a_plain_word_searches_as_it_is_typed_under_the_search_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Archive plan",
                "Boxes for the move.",
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
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Archive plan"]).await,
            "typing searched nothing: {:?}",
            bar.result_subjects()
        );
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line == "Archive"),
            "the command: {said:?}"
        );
        let search = said
            .iter()
            .position(|line| line.starts_with("Search mail for"))
            .expect("the search row stays");
        let heading = said
            .iter()
            .position(|line| line.starts_with("Conversations"))
            .expect("the results' heading");
        assert!(
            search < heading,
            "the search row is above the results: {said:?}"
        );
        assert!(
            said.iter().any(|line| line == "Boxes for the move."),
            "a hit shows the message's first line: {said:?}"
        );
        // A plain word is what the entry already shows; it is not a chip.
        assert!(
            bar.chips().is_empty(),
            "a plain word chipped: {:?}",
            bar.chips()
        );
        // The command named by what was typed is under the cursor; Return
        // on the search row hands the keyboard to the first hit.
        assert_eq!(bar.highlighted().map(|(kind, _, _)| kind), Some("command"));
        bar.run_search();
        assert_eq!(
            bar.highlighted().map(|(kind, _, _)| kind),
            Some("message"),
            "Return on the search row did not reach the first hit"
        );
    });
}

/// A word no command is named by, which one command merely contains the
/// letters of in order, is a search: the search row is what Return runs.
pub fn a_word_no_command_is_named_by_searches_first() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Tide tables", "x", 5)
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || support::subjects(&window).len() == 1).await);
        let bar = open_bar(&window);
        type_in(&bar, "tide").await;
        crate::settle();
        assert_eq!(
            bar.highlighted().map(|(kind, _, _)| kind),
            Some("search"),
            "Return would run {:?}",
            bar.highlighted()
        );
    });
}

/// `@` offers the correspondents the address book holds, and choosing one
/// puts their `from:` search in the box.
pub fn at_offers_correspondents() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.correspondent("Ada Moreno", "ada@example.com").await;
        fixture.correspondent("Lena Park", "lena@example.org").await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Hello", "x", 5)
            .await;
        let (window, _client) = fixture.open().await;
        let bar = open_bar(&window);
        assert!(crate::settle_until(async || bar.places_known()).await);
        type_in(&bar, "@").await;
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line == "Ada Moreno")
                && said.iter().any(|line| line == "lena@example.org"),
            "no correspondents offered: {said:?}"
        );
        type_in(&bar, "@ada").await;
        let said = bar.texts();
        assert!(!said.iter().any(|line| line == "Lena Park"), "{said:?}");
        bar.run_search_row();
        assert_eq!(bar.typed(), "from:ada@example.com");
    });
}

/// With two accounts the same message filed in each is two rows, and each
/// row says which account it is in.
pub fn a_result_names_its_account_when_there_are_two() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Harbour lights", "x", 5)
            .await;
        let (second, inbox) = fixture.second_account().await;
        fixture.file_as(second.id, inbox, "Harbour lights", 6).await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        let bar = open_bar(&window);
        assert!(crate::settle_until(async || bar.places_known()).await);
        type_in(&bar, "harbour").await;
        assert!(
            crate::settle_until(async || bar.result_subjects().len() == 2).await,
            "{:?}",
            bar.result_subjects()
        );
        let said = bar.texts().join(" | ");
        assert!(said.contains("second@example.org"), "{said}");
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

async fn three_and_open() -> postio_gtk::window::FocusWindow {
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
        // ADR 0037: one row names the typed word, and it is the offer; the
        // search row names what the results under it are for.
        let naming_typed = said
            .iter()
            .filter(|line| line.starts_with("Search") && line.contains("qarterly"))
            .count();
        assert_eq!(
            naming_typed, 1,
            "two rows name the typed word, so Return's query is unclear: {said:?}"
        );
        assert!(
            said.iter()
                .any(|line| line == "Search mail for \u{201c}quarterly\u{201d}"),
            "the search row does not name the results' query: {said:?}"
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

/// gtk-design: in a result the first line gives way before the subject.
/// A subject of ordinary length, under a long first line, is drawn whole at
/// 1280 px.
pub fn a_result_gives_its_subject_room_before_its_first_line() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let subject = "[harbour-dev] Tide gate interlock";
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                subject,
                "The proposal below changes how the tide gate interlock is armed during maintenance windows and why.",
                5,
            )
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open_sized(Some((1280, 800))).await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "harbour").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == [subject]).await,
            "typing searched nothing: {:?}",
            bar.result_subjects()
        );
        let label = support::only(bar.widget(), "focus-bar-subject")
            .downcast::<gtk::Label>()
            .expect("the subject is a label");
        assert!(
            crate::settle_until(async || label.width() > 0).await,
            "the result was never laid out"
        );
        assert!(
            !label.layout().is_ellipsized(),
            "the subject was cut ({} px) while the first line kept its room",
            label.width()
        );
    });
}

/// `O` is a letter in the box, whatever is highlighted; the order row is
/// what switches the results between relevance and date, with a click or
/// Return, and a row says the same to a mouse.
pub fn o_is_a_letter_and_the_order_row_switches_the_results() {
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
        // The older one's subject says the word and the newer one's only
        // its body: a subject that says all of the query outranks a passing
        // mention, while two that both say it are as good as each other and
        // the newer leads -- so this pair is what disagrees with date order.
        let (dense, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Quarterly report",
                "Report.",
                600,
            )
            .await;
        fixture
            .write_searchable_body(dense, "report report report report report")
            .await;
        let (glancing, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Notes",
                "One report among other things.",
                300,
            )
            .await;
        fixture
            .write_searchable_body(glancing, "One report among other things entirely")
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        // The list has landed. `subjects` is the rows on screen, fewer than
        // the twenty-two filed, so waiting for twenty sat out the deadline.
        assert!(
            crate::settle_until(async || !support::subjects(&window).is_empty()).await,
            "the inbox never listed its mail"
        );
        let bar = open_bar(&window);
        type_in(&bar, "report").await;
        let relevance = vec!["Quarterly report".to_owned(), "Notes".to_owned()];
        let by_date = vec!["Notes".to_owned(), "Quarterly report".to_owned()];
        assert!(
            crate::settle_until(async || bar.result_subjects().len() == 2).await,
            "no results: {:?}",
            bar.result_subjects()
        );
        assert_eq!(
            bar.result_subjects(),
            relevance,
            "relevance puts the denser match first"
        );
        // Typing wins, with a result chosen or not: `O` is a letter.
        support::press(&window, "Down", gtk::gdk::ModifierType::empty());
        assert!(
            !support::deliver(&window, "O"),
            "O was claimed from the box"
        );
        assert_eq!(
            bar.result_subjects(),
            relevance,
            "a typed O reordered the rows"
        );
        // `alt+o` switches it while the query has the keyboard.
        assert!(
            support::deliver_with(&window, "o", gtk::gdk::ModifierType::ALT_MASK),
            "alt+o was not taken"
        );
        assert!(
            crate::settle_until(async || bar.result_subjects() == by_date).await,
            "alt+o did not reorder: {:?}",
            bar.result_subjects()
        );
        assert_eq!(bar.typed(), "report", "alt+o typed nothing");
        // The row says the key.
        assert!(
            bar.texts().iter().any(|line| line == "alt+o"),
            "the order row shows no key: {:?}",
            bar.texts()
        );
        assert!(
            bar.texts().iter().any(|line| line == "Sorted by date"),
            "the bar does not say its order: {:?}",
            bar.texts()
        );
        support::click_row_saying(&window, bar.widget(), "Sorted by date");
        assert!(
            crate::settle_until(async || bar.result_subjects() == relevance).await,
            "the order row did not switch back: {:?}",
            bar.result_subjects()
        );
    });
}

/// A message opened from search steps the results it came from: `j` and `k`
/// walk the bar's hits, "Result n of m" says where, `[` and `]` step the
/// hit's own thread, and Escape returns to the results with the bar as it
/// was.
pub fn a_hit_steps_the_results_and_its_own_thread() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Harbor budget",
                "Numbers for harbor.",
                5,
            )
            .await;
        fixture.thread_of("Harbor draft", 3, 20).await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "harbor").await;
        assert!(
            crate::settle_until(async || bar.result_subjects().len() >= 2).await,
            "the search found {:?}",
            bar.result_subjects()
        );
        let hits = bar.result_subjects();
        bar.run_search();
        assert_eq!(
            bar.highlighted().map(|(kind, _, _)| kind),
            Some("message"),
            "{:?}",
            bar.texts()
        );
        chain(&window, "Return");
        assert!(
            crate::settle_until(async || window.reading().is_some_and(|r| r.is_open())).await,
            "the hit never opened"
        );
        let reading = window.reading().expect("the hit opened");
        assert!(
            crate::settle_until(async || reading.is_open() && reading.title() == hits[0]).await,
            "the first hit did not open: {:?}",
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

        // The hit's own thread: step to the three-message conversation and
        // through it with `[` and `]`. Its row is whichever of its messages
        // ranked first -- all three say "harbor", so the newest, a reply.
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
        // Toward the middle message, from whichever end the hit opened at.
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message ")).await,
            "the hit's body never drew: {:?}",
            reading.body_text()
        );
        let toward_middle = if reading.body_text().contains("Message 1") {
            "bracketright"
        } else {
            "bracketleft"
        };
        chain(&window, toward_middle);
        assert!(
            crate::settle_until(async || reading.body_text().contains("Message 2")).await,
            "{toward_middle} did not step the hit's thread: {:?}",
            reading.body_text()
        );
        assert!(
            reading.subtitle().contains("2 of 3 in the thread"),
            "{:?}",
            reading.subtitle()
        );

        // Escape closes the message and the results are back.
        chain(&window, "Escape");
        assert!(
            crate::settle_until(async || {
                window.bar().is_some_and(|bar| bar.is_open())
                    && bar.result_subjects().len() == hits.len()
            })
            .await,
            "the results did not come back: {:?}",
            window.bar().map(|bar| bar.result_subjects())
        );
    });
}

/// A key pressed along the real focus chain, as a storyboard does.
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

/// Walking the results with the arrows keeps the highlighted row in view:
/// the highlight went below the bottom of the results and the list stayed
/// where it was, so the row Return would open could not be seen.
pub fn the_arrows_scroll_the_results_to_the_highlighted_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for n in 0..25 {
            fixture
                .file(
                    ("Ada Moreno", "ada@example.com"),
                    &format!("Standup notes {n}"),
                    "Who is doing what.",
                    n + 1,
                )
                .await;
        }
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        let bar = open_bar(&window);
        type_in(&bar, "standup").await;
        assert!(
            crate::settle_until(async || bar.result_subjects().len() == 25).await,
            "standup did not list the 25: {:?}",
            bar.result_subjects()
        );
        let list = support::with_class(bar.widget(), "focus-bar-results")
            .into_iter()
            .next()
            .and_downcast::<gtk::ListBox>()
            .expect("the results list");
        let scroller = list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the results scroll");
        assert!(
            crate::settle_until(async || scroller.height() > 0).await,
            "the results never took a height"
        );
        assert!(
            crate::settle_until(async || list.height() > scroller.height() + 100).await,
            "the results must overflow for this to mean anything: list {} in a scroller of {}",
            list.height(),
            scroller.height()
        );
        // Whether the highlighted row lies wholly inside what the scroller
        // shows.
        let in_view = || {
            let Some(row) = list.selected_row() else {
                return false;
            };
            let Some(bounds) = row.compute_bounds(&scroller) else {
                return false;
            };
            bounds.y() >= -0.5 && bounds.y() + bounds.height() <= scroller.height() as f32 + 0.5
        };
        for step in 1..=24 {
            bar.press(gtk::gdk::Key::Down, gtk::gdk::ModifierType::empty());
            assert!(
                crate::settle_until(async || in_view()).await,
                "after {step} presses of Down the highlighted row is out of view"
            );
        }
        for step in 1..=24 {
            bar.press(gtk::gdk::Key::Up, gtk::gdk::ModifierType::empty());
            assert!(
                crate::settle_until(async || in_view()).await,
                "after {step} presses of Up the highlighted row is out of view"
            );
        }
    });
}
