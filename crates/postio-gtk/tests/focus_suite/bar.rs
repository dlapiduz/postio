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

/// Spec 010 US7: `in:Rec` lists the folders it could mean, Receipts with
/// its count; Return makes its chip, and the dropdown's hits are Receipts'
/// mail.
pub fn in_rec_offers_receipts_and_return_searches_it() {
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
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the inbox never reached the screen"
        );
        let bar = open_bar(&window);
        type_in(&bar, "in:Rec").await;
        assert!(
            crate::settle_until(async || {
                bar.highlighted()
                    .is_some_and(|(kind, _, text)| kind == "folder" && text.starts_with("Receipts"))
            })
            .await,
            "Receipts is not offered: {:?} of {:?}",
            bar.highlighted(),
            bar.dropdown().texts()
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || bar.typed().trim_end() == "in:Receipts").await,
            "Return did not make the chip: {:?}",
            bar.typed()
        );
        assert!(
            crate::settle_until(async || {
                let mut hits = bar.result_subjects();
                hits.sort();
                hits == ["Bookshop order", "Coffee beans", "Train ticket"]
            })
            .await,
            "not Receipts' mail: {:?}",
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
        // The empty field lists the saved searches as pills, each with its
        // key; a click on one runs it, as its key does.
        bar.set_text("");
        assert!(
            crate::settle_until(async || {
                bar.dropdown()
                    .pills()
                    .iter()
                    .any(|pill| pill.contains("From Ada") && pill.contains("alt+2"))
            })
            .await,
            "the saved pill does not name it with its key: {:?}",
            bar.dropdown().pills()
        );
        let first = bar.dropdown().pills().first().cloned().expect("a pill");
        assert!(
            bar.dropdown().click_pill(&first),
            "{first:?} could not be pressed"
        );
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
        // A keystroke that makes a search is one conversation search, as it
        // is typed: not every keystroke makes one -- a single letter is in
        // most of a mailbox and is not a search yet
        // (`ParsedQuery::is_searchable`). One to three letters also ask what
        // they could become (spec 010 US7), and the hits that land ask for
        // their passages. With no digest rule nothing is asked about held
        // mail, and nothing else is asked at all.
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

/// Spec 010 US1: a word lists its top hits under the field, each with its
/// folder; Show all is what the highlight starts on, so Return opens the
/// results. A word in the search's field is never a command: the commands
/// are `>`'s and Ctrl+K's.
pub fn a_word_lists_top_hits_and_return_shows_all() {
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
        type_in(&bar, "archive").await;
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Archive plan"]).await,
            "typing searched nothing: {:?}",
            bar.texts()
        );
        let said = bar.texts();
        assert!(
            said.iter().any(|line| line == "in:Inbox"),
            "the hit names its folder: {said:?}"
        );
        assert!(
            !said.iter().any(|line| line == "Archive"),
            "a word listed a command: {said:?}"
        );
        assert_eq!(
            bar.highlighted().map(|(kind, _, _)| kind),
            Some("show_all"),
            "{:?}",
            bar.highlighted()
        );
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.results().is_some()).await,
            "Return did not open the results"
        );
        assert!(!bar.is_open(), "the bar stayed up over the results");
    });
}

/// `@` offers the correspondents the address book holds, and choosing one
/// puts their `with:` search in the box: mail involving any of the person's
/// addresses, as specs/005-contacts/contracts/query-with.md has the finder do.
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
        assert_eq!(bar.typed(), "with:ada@example.com");
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

/// Spec 010 US7: plain English is shown as what it was understood as, a
/// tile a term; `Ctrl+Backspace` keeps the words as words; `Tab` makes the
/// tiles chips in the field; `Ctrl+S` saves the query to `config.toml`,
/// pinned, and the empty field's pills show it with its key.
pub fn plain_english_is_understood_tab_makes_chips_and_ctrl_s_saves() {
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
        let understood = || bar.dropdown().understood();
        assert!(
            crate::settle_until(async || understood().iter().any(|tile| tile.contains("from:")))
                .await,
            "the words were not understood: {:?}",
            understood()
        );

        support::press(&window, "BackSpace", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || understood().is_empty()).await,
            "Ctrl+Backspace kept the tiles: {:?}",
            understood()
        );
        assert_eq!(bar.typed(), words, "the field keeps the words");

        bar.set_text(&format!("{words} "));
        assert!(
            crate::settle_until(async || !understood().is_empty()).await,
            "understood again: {:?}",
            understood()
        );
        assert!(
            support::deliver(&window, "Tab"),
            "Tab reached the bar: {}",
            support::focus_path(&window)
        );
        assert!(
            crate::settle_until(async || bar.typed().starts_with("invoices from:")).await,
            "Tab did not make chips: {:?}",
            bar.typed()
        );
        let query = bar.typed().trim().to_owned();

        support::press(&window, "s", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains(&format!("query = \"{query}\""))
            })
            .await,
            "Ctrl+S saved no {query:?}:\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("# Mine."),
            "the rest of the file is kept"
        );
        bar.set_text("");
        assert!(
            crate::settle_until(async || {
                bar.dropdown()
                    .pills()
                    .iter()
                    .any(|pill| pill.contains("alt+1"))
            })
            .await,
            "the saved pill does not show it: {:?}",
            bar.dropdown().pills()
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

/// T241, ADR 0037 as amended 2026-10-08: the bar's search forgives, so a
/// misspelled word finds the mail that was meant, after anything it finds
/// exactly; quoted, the word is exactly itself, which is the way out.
pub fn a_misspelled_word_finds_what_was_meant_and_quoted_is_exact() {
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
        assert!(
            crate::settle_until(async || bar.result_subjects() == ["Quarterly planning"]).await,
            "the word that was meant was not found: {:?}",
            bar.result_subjects()
        );

        bar.set_text("\"qarterly\"");
        assert!(
            crate::settle_until(async || bar.result_subjects().is_empty()).await,
            "the quoted word still lists the mail it does not hold: {:?}",
            bar.result_subjects()
        );
    });
}

/// gtk-design: in a hit the passage gives way before the subject. A
/// subject of ordinary length, beside a long passage, is drawn whole at
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
                "The harbour proposal below changes how the tide gate interlock is armed during maintenance windows and why.",
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
        // Found again each time: the passages landing draws the rows anew.
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

/// `O` is a letter in the field, whatever is highlighted; `alt+o` is the
/// results' Sort, between best match and newest, and the button says which.
pub fn o_is_a_letter_and_alt_o_switches_the_results_sort() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Quarterly report",
                "Report.",
                600,
            )
            .await;
        let (notes, _) = fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Notes",
                "One report among other things.",
                300,
            )
            .await;
        fixture
            .write_searchable_body(notes, "One report among other things")
            .await;
        fixture.index().await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the inbox never listed its mail"
        );
        let bar = open_bar(&window);
        type_in(&bar, "report").await;
        assert!(
            crate::settle_until(async || bar.result_subjects().len() == 2).await,
            "no hits: {:?}",
            bar.texts()
        );
        // Typing wins, with a hit highlighted or not: `O` is a letter.
        support::press(&window, "Up", gtk::gdk::ModifierType::empty());
        assert!(
            !support::deliver(&window, "O"),
            "O was claimed from the field"
        );
        bar.set_text("report");
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.results().is_some()).await,
            "Ctrl+Return did not open the results"
        );
        let results = window.results().expect("the results");
        let best = results.sort_said();
        assert!(best.contains("Best match"), "{best:?}");
        assert!(
            support::deliver_with(&window, "o", gtk::gdk::ModifierType::ALT_MASK),
            "alt+o was not taken"
        );
        assert!(
            crate::settle_until(async || results.sort_said().contains("Newest")).await,
            "alt+o did not switch the sort: {:?}",
            results.sort_said()
        );
        assert!(
            support::deliver_with(&window, "o", gtk::gdk::ModifierType::ALT_MASK),
            "alt+o was not taken again"
        );
        assert!(
            crate::settle_until(async || results.sort_said() == best).await,
            "alt+o did not switch back: {:?}",
            results.sort_said()
        );
    });
}

/// A message opened from the results steps the results it came from: `j`
/// and `k` walk them, "Result n of m" says where, `n` and `p` step the
/// result's own thread, and Escape returns to the results.
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
        support::press(&window, "Return", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.results().is_some()).await,
            "Show all did not open the results"
        );
        let results = window.results().expect("the results");
        // Every row, in the results' own order: Top hits, then each of them
        // again under its month. The walk is the rows, as drawn.
        assert!(
            crate::settle_until(async || results.subjects().len() == 4).await,
            "the results found {:?}",
            results.subjects()
        );
        let hits = results.subjects();
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
            reading.subtitle().contains("2 of 3 in the thread"),
            "{:?}",
            reading.subtitle()
        );

        // Escape closes the message and the results are back.
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

/// Walking the results with the keys keeps the ringed row in view: the
/// ring went below the bottom of the list and the list stayed where it was,
/// so the row Return would open could not be seen.
pub fn walking_the_results_keeps_the_ringed_row_in_view() {
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
        // Whether the ringed row lies wholly inside what the scroller shows.
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
