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
        {
            use postio_ui::observe::{Overlay, Region};
            let seen = window.observe();
            assert_eq!(
                seen.keyboard.region,
                Region::Dialog,
                "the sheet holds the keyboard, not the dimmed list: {seen:#?}"
            );
            assert_eq!(seen.overlay.kind, Overlay::Dialog);
        }
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

/// Marking a row with `x` ticks it and leaves the list where it was.
pub fn x_marks_a_row_without_moving_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        for n in 0..40 {
            fixture
                .correspondent(&format!("Person {n:02}"), &format!("p{n:02}@example.test"))
                .await;
        }
        fixture
            .file(("Person 00", "p00@example.test"), "Hello", "Hi.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window
                .contacts()
                .is_some_and(|view| view.total() == 40 && view.cursor() == Some(0)))
            .await
        );
        let view = window.contacts().expect("the view");
        support::keys(&window, &["j", "j", "j", "j", "j", "j", "j", "j", "j", "j"]);
        assert!(
            crate::settle_until(async || view.cursor() == Some(10)
                && view.card_name().is_some()
                && view.card_name() == view.name_at(10))
            .await,
            "the cursor is on the tenth person and its card has been read"
        );
        crate::settle();
        let before = view.scroll_offset();
        support::keys(&window, &["x"]);
        assert!(crate::settle_until(async || view.marked_count() == 1).await);
        crate::settle();
        assert_eq!(view.scroll_offset(), before, "marking moved the list");
    });
}

/// `i` on a row opens the popover at the sender: who they are, the arrows
/// choose where their mail arrives, and Escape puts it away (handoff 04).
pub fn i_opens_the_popover_the_arrows_choose_where_mail_arrives_and_escape_closes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
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
        // z is the toast's Undo and takes the choice back, in the popover too.
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

/// A sender nobody has saved gets the popover that says how they reached
/// you, with Add contact as its main action (handoff 05).
pub fn a_sender_not_yet_saved_is_introduced_and_offered_add_contact() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Jonas Weber", "jonas@harbor.test"), "Intro", "Hello.", 5)
            .await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || !support::subjects(&window).is_empty()).await);
        support::keys(&window, &["i"]);
        assert!(
            crate::settle_until(async || window
                .contact_popover()
                .is_some_and(|view| view.intro.is_some()))
            .await,
            "a new sender is introduced"
        );
        let view = window.contact_popover().expect("a popover");
        assert!(
            view.actions
                .iter()
                .any(|action| action.label.starts_with("Add") && action.primary),
            "{:?}",
            view.actions
        );
    });
}

/// `v i` asks for a `.vcf`, reads the one it is given, says what it did,
/// merges nothing, and puts the cursor on the first person it made; `v x`
/// writes one file of the people the list holds and says how many.
pub fn v_i_imports_a_chosen_file_and_v_x_exports_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        use postio_gtk::chooser::SavePick;
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        let directory = tempfile::tempdir().expect("a directory");
        let cards = directory.path().join("cards.vcf");
        std::fs::write(
            &cards,
            "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:zed\r\nFN:Zed Quinn\r\nEMAIL:zed@example.test\r\nEND:VCARD\r\n",
        )
        .expect("a card");
        let exported = directory.path().join("Contacts.vcf");
        {
            let (cards, exported) = (cards.clone(), exported.clone());
            window.set_file_picker(move |pick, answer| match pick {
                SavePick::Cards => answer(Some(cards.clone())),
                SavePick::CardsFile { .. } => answer(Some(exported.clone())),
                _ => answer(None),
            });
        }
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window.contacts().is_some_and(|view| view.total() == 3))
                .await
        );
        support::keys(&window, &["v", "i"]);
        assert!(
            crate::settle_until(async || window
                .contacts_io_sheet()
                .is_some_and(|sheet| sheet.title == "Imported 1 contact"))
            .await,
            "v i reads the chosen file and says so"
        );
        let sheet = window.contacts_io_sheet().expect("a sheet");
        assert!(
            sheet
                .lines
                .last()
                .is_some_and(|line| line.contains("cannot be undone")),
            "{:?}",
            sheet.lines
        );
        let view = window.contacts().expect("the view");
        assert!(
            crate::settle_until(async || view.total() == 4
                && view.cursor().and_then(|at| view.name_at(at)).as_deref() == Some("Zed Quinn"))
            .await,
            "the cursor is on the first person the import made"
        );
        window.close_dialogs_for_test();

        support::keys(&window, &["v", "x"]);
        assert!(
            crate::settle_until(async || exported.exists()
                && window
                    .contacts_io_sheet()
                    .is_some_and(|sheet| sheet.title.starts_with("Exported")))
            .await,
            "v x writes the file and says how many"
        );
        let written = std::fs::read_to_string(&exported).expect("the file");
        assert!(
            written.contains("BEGIN:VCARD") && written.contains("zed@example.test"),
            "{written}"
        );
    });
}

/// Settings > Contacts: the four Suggestions switches, in the controller's
/// words, each turning its work off at the source -- a flip reaches the
/// store and comes back as the redrawn pane (handoff screen 08, less sync,
/// which is the Mac's).
pub fn settings_contacts_flips_the_suggestions_and_says_nothing_of_sync() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        use postio_model::Suggestion;
        use postio_ui::settings::Section;
        let fixture = fixture().await;
        let (window, client) = fixture.open().await;
        window.act(postio_core::CommandId::Settings);
        assert!(
            crate::settle_until(async || window
                .settings()
                .is_some_and(|settings| settings.is_open()))
            .await
        );
        let panel = window.settings().expect("settings").panel().clone();
        panel.show_section(Section::Contacts);
        assert!(
            crate::settle_until(async || panel.suggestions_shown().is_some()).await,
            "the pane asks which switches are on"
        );
        let view = panel.suggestions_shown().expect("the section");
        let on: Vec<bool> = view.rows.iter().map(|row| row.on).collect();
        assert_eq!(on, [false, true, true, true]);
        let said = panel.contacts_texts();
        assert!(
            said.iter()
                .any(|line| line == "Save people I write to, automatically"),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|line| line.contains("run on this computer")),
            "{said:?}"
        );
        assert!(
            !said.iter().any(|line| line.to_lowercase().contains("sync")),
            "sync is the Mac's: {said:?}"
        );

        // "Read signatures for details" off: the store says so and the pane
        // is drawn from the answer.
        panel.test_flip_suggestion(2);
        assert!(
            crate::settle_until(async || !client
                .contact_suggestions()
                .await
                .expect("the switches")
                .is_on(Suggestion::ReadSignatures))
            .await,
            "the flip reached the store"
        );
        assert!(
            crate::settle_until(async || panel
                .suggestions_shown()
                .is_some_and(|view| !view.rows[2].on))
            .await,
            "the pane is redrawn from the host's answer"
        );
    });
}

/// Return on a person is "all mail with them": the view goes home and the
/// box is open on a search for their addresses.
pub fn return_searches_the_mail_of_the_person_under_the_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "c"]);
        assert!(
            crate::settle_until(async || window
                .contacts()
                .is_some_and(|view| view.card_name().as_deref() == Some("Ada Moreno")))
            .await
        );
        support::keys(&window, &["Return"]);
        assert!(
            crate::settle_until(async || window.contacts().is_none()
                && window
                    .bar()
                    .is_some_and(|bar| bar.is_open() && bar.query().contains("ada@example.test")))
            .await,
            "Return closes Contacts and opens the box on their address"
        );
    });
}
