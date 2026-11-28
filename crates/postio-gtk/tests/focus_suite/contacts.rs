use crate::support::{self, Fixture};

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

pub fn g_c_opens_the_people_j_k_walk_them_the_card_follows_and_escape_goes_home() {
    crate::gtk_case(async {
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

pub fn the_strip_says_its_tabs_and_a_click_on_one_is_the_key() {
    crate::gtk_case(async {
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

pub fn ctrl_e_edits_escape_arms_before_it_discards_and_ctrl_s_saves() {
    crate::gtk_case(async {
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

        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || view
                .form_texts()
                .iter()
                .any(|line| line.contains("Escape again")))
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

pub fn i_opens_the_popover_the_arrows_choose_where_mail_arrives_and_escape_closes() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture
            .correspondent("Ada Moreno", "ada@example.test")
            .await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.test"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || !support::subjects(&window).is_empty()).await);
        support::keys(&window, &["i"]);
        assert!(
            crate::settle_until(async || window.contact_popover().is_some()).await,
            "i opens the popover"
        );
        let view = window.contact_popover().expect("a popover");
        assert_eq!(view.name, "Ada Moreno");
        assert_eq!(view.address, "ada@example.test");
        assert!(
            window
                .contact_popover_texts()
                .iter()
                .any(|line| line == "Ada Moreno"),
            "{:?}",
            window.contact_popover_texts()
        );
        let on = |window: &postio_gtk::window::FocusWindow| {
            window.contact_popover().and_then(|view| {
                view.arrival
                    .iter()
                    .find(|option| option.on)
                    .map(|o| o.label.clone())
            })
        };
        assert_eq!(on(&window).as_deref(), Some("Inbox"));
        support::keys(&window, &["Right"]);
        assert!(
            crate::settle_until(async || on(&window).as_deref() == Some("Digest")).await,
            "the right arrow chooses the next place mail arrives"
        );
        assert!(
            crate::settle_until(async || window
                .toast_showing()
                .is_some_and(|said| said.contains("arrives")))
            .await,
            "the change is said, with its Undo"
        );
        support::keys(&window, &["z"]);
        assert!(
            crate::settle_until(async || on(&window).as_deref() == Some("Inbox")).await,
            "undo puts the earlier arrival back and the popover says so"
        );
        support::keys(&window, &["Escape"]);
        assert!(
            crate::settle_until(async || window.contact_popover().is_none()).await,
            "Escape puts it away"
        );
        assert!(
            !support::subjects(&window).is_empty(),
            "the list is as it was"
        );
    });
}

pub fn the_card_lists_the_contacts_keys_and_where_each_came_from() {
    crate::gtk_case(async {
        let fixture = fixture().await;
        fixture
            .write_key("ada@example.test", "E8DF9987513ECF47")
            .await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || {
                let shown = support::texts(&window);
                shown.iter().any(|text| text == "E8DF 9987 513E CF47")
                    && shown.iter().any(|text| text == "From your GnuPG keyring")
            })
            .await,
            "Ada's card names her key: {:?}",
            support::texts(&window)
        );
    });
}
