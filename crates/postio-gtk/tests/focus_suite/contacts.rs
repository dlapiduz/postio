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
