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
