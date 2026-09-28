//! The command bar (US4, T086; screens 07-09): `/` opens one bar for
//! search, places and commands.

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
