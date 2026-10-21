//! The Contacts view (specs/011-focus-contacts S13, handoff screens 01 to
//! 03), the keyboard's way: `g c` puts the people where the inbox was, `j`
//! and `k` walk them with the card following, `]` changes the kind tab, `/`
//! filters, and Escape goes home one rung at a time. What the person sees is
//! the widget tree's: the names in the list, the card's name, the tabs.

use crate::support::{self, Fixture};

/// Three people, as sync's catch-up leaves them.
async fn fixture() -> Fixture {
    let fixture = Fixture::empty().await;
    for (name, address) in [
        ("Ada Moreno", "ada@example.test"),
        ("Ben Adeyemi", "ben@example.test"),
        ("Grace Oyelaran", "grace@example.test"),
    ] {
        fixture.correspondent(name, address).await;
        fixture.file((name, address), "Hello", "Hi.", 5).await;
    }
    fixture
}

/// `g c`, `j`, `k` and Escape: the people, in letter order, the cursor on the
/// first, the card of whoever has it, and the inbox back as it was.
pub fn g_c_opens_the_people_j_k_walk_them_the_card_follows_and_escape_goes_home() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || {
                window
                    .contacts()
                    .is_some_and(|view| view.total() == 3 && view.card_name().is_some())
            })
            .await,
            "the view opens with its three people and the first one's card"
        );
        let view = window.contacts().expect("the view");
        assert_eq!(
            view.names(3),
            ["Ada Moreno", "Ben Adeyemi", "Grace Oyelaran"]
        );
        assert_eq!(view.cursor(), Some(0));
        assert_eq!(view.card_name().as_deref(), Some("Ada Moreno"));
        assert!(view.status().contains("3 contacts"), "{}", view.status());

        support::keys(&window, &["j"]);
        assert!(
            crate::settle_until(async || view.cursor() == Some(1)
                && view.card_name().as_deref() == Some("Ben Adeyemi"))
            .await,
            "j moves the cursor and the card follows"
        );
        support::keys(&window, &["k"]);
        assert!(
            crate::settle_until(async || view.cursor() == Some(0)
                && view.card_name().as_deref() == Some("Ada Moreno"))
            .await
        );

        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.contacts().is_none()).await,
            "Escape with nothing typed goes home"
        );
        assert!(
            !support::subjects(&window).is_empty(),
            "the inbox is back with its rows"
        );
    });
}

/// The tabs are the strip's, in the strip's words: All first and chosen, a
/// click on one is the key `]` would have been.
pub fn the_strip_says_its_tabs_and_a_click_on_one_is_the_key() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window.contacts().is_some_and(|view| view.total() == 3))
                .await
        );
        let view = window.contacts().expect("the view");
        let tabs = view.tabs();
        assert_eq!(tabs.first(), Some(&("All".to_owned(), true)), "{tabs:?}");
        assert!(
            tabs.iter().any(|(title, _)| title == "Companies"),
            "{tabs:?}"
        );

        view.test_click_tab("Companies");
        assert!(
            crate::settle_until(async || {
                window
                    .contacts()
                    .is_some_and(|view| view.tabs().iter().any(|(t, on)| t == "Companies" && *on))
            })
            .await,
            "the chosen tab is Companies"
        );
        assert!(
            crate::settle_until(async || window.contacts().is_some_and(|view| view.total() == 0))
                .await,
            "nobody here is a company"
        );
        support::keys(&window, &["bracketleft"]);
        assert!(
            crate::settle_until(async || {
                window.contacts().is_some_and(|view| view.total() == 3)
            })
            .await,
            "[ goes back a tab"
        );
    });
}

/// `/` puts the keyboard in the filter, what is typed narrows the list, and
/// Escape clears it before it closes anything.
pub fn slash_filters_the_list_and_escape_clears_before_it_closes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window.contacts().is_some_and(|view| view.total() == 3))
                .await
        );
        let view = window.contacts().expect("the view");
        support::keys(&window, &["slash"]);
        assert!(view.filter_has_focus(), "/ puts the keyboard in the filter");
        view.test_type_filter("gra");
        assert!(
            crate::settle_until(async || view.names(5) == ["Grace Oyelaran"]).await,
            "only Grace matches"
        );
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || view.filter_text().is_empty() && view.total() == 3).await,
            "Escape clears the filter and the list comes back"
        );
        assert!(window.contacts().is_some(), "the view is still up");
        support::keys(&window, &["Escape"]);
        assert!(crate::settle_until(async || window.contacts().is_none()).await);
    });
}

/// `ctrl+e` opens the form over the card, a typed field comes back drawn from
/// what the controller answers, the first Escape arms and the second
/// discards, and `ctrl+s` saves what was typed (handoff screen 06).
pub fn ctrl_e_edits_escape_arms_before_it_discards_and_ctrl_s_saves() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        use postio_gtk::contacts::Slot;
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window
                .contacts()
                .is_some_and(|view| view.card_name().as_deref() == Some("Ada Moreno")))
            .await
        );
        let view = window.contacts().expect("the view");
        support::press(&window, "e", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || view.has_form()).await,
            "ctrl+e opens the form"
        );
        assert!(
            view.form_texts()
                .iter()
                .any(|line| line.starts_with("Editing")),
            "{:?}",
            view.form_texts()
        );
        view.test_type_form(Slot::Title, "Finance lead");
        assert!(
            crate::settle_until(
                async || view.form_text(Slot::Title).as_deref() == Some("Finance lead")
            )
            .await,
            "the field is drawn from the controller's answer"
        );

        // The first Escape arms, and says so.
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || view
                .form_texts()
                .iter()
                .any(|line| line.contains("Esc again")))
            .await,
            "{:?}",
            view.form_texts()
        );
        assert!(view.has_form(), "armed, not discarded");
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || !view.has_form()).await,
            "the second Escape discards"
        );

        // And saving keeps it.
        support::press(&window, "e", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(crate::settle_until(async || view.has_form()).await);
        view.test_type_form(Slot::Title, "Finance lead");
        assert!(
            crate::settle_until(
                async || view.form_text(Slot::Title).as_deref() == Some("Finance lead")
            )
            .await
        );
        support::press(&window, "s", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !view.has_form()
                && view
                    .card()
                    .is_some_and(|card| card.role.contains("Finance lead")))
            .await,
            "ctrl+s saves, and the card says what was typed"
        );
    });
}

/// Two records of one person: `g w` opens the merge sheet with its reasons,
/// Return merges and the list says one person where it said two, and
/// Escape is Later.
pub fn g_w_offers_the_look_alikes_return_merges_and_escape_is_later() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .correspondent("Ada Moreno", "ada@example.test")
            .await;
        fixture
            .correspondent("ada  moreno", "a.moreno@example.net")
            .await;
        fixture
            .correspondent("Ben Adeyemi", "ben@example.test")
            .await;
        fixture
            .file(("Ben Adeyemi", "ben@example.test"), "Hello", "Hi.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "w"]);
        assert!(
            crate::settle_until(async || window
                .contacts()
                .is_some_and(|view| !view.merge_texts().is_empty()))
            .await,
            "g w opens the sheet"
        );
        let view = window.contacts().expect("the view");
        let said = view.merge_texts();
        assert!(said.iter().any(|line| line == "Not the same"), "{said:?}");
        assert!(said.iter().any(|line| line == "Later"), "{said:?}");
        assert!(said.iter().any(|line| line == "Merge"), "{said:?}");
        assert_eq!(view.total(), 3);

        // Later puts it away and leaves the people as they were.
        support::keys(&window, &["Escape"]);
        assert!(crate::settle_until(async || view.merge_texts().is_empty()).await);
        assert_eq!(view.total(), 3);

        // Again, and merge.
        support::keys(&window, &["g", "w"]);
        assert!(crate::settle_until(async || !view.merge_texts().is_empty()).await);
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || view.total() == 2 && view.merge_texts().is_empty()).await,
            "two records became one person"
        );
    });
}
