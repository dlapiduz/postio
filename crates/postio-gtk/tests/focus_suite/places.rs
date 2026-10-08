//! The folders popover (US4 scenario 6, T088; screen 10): `g o` lists the
//! places to go, typing filters them, and `Enter` goes to the first one --
//! the list shows it, and the header names it.

use crate::support::{self, Fixture};

pub fn g_o_then_trav_and_enter_shows_travel() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Inbox mail", "Hello.", 5)
            .await;
        let travel = fixture.folder("Travel").await;
        fixture.file_in(travel, "Train to the coast", 60).await;
        fixture.file_in(travel, "Hotel booking", 20).await;
        let receipts = fixture.folder("Receipts").await;
        fixture.file_in(receipts, "Coffee beans", 30).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "the inbox never reached the screen"
        );

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Travel".to_owned())).await,
            "the popover never listed Travel: {:?}",
            places.names()
        );
        assert!(
            places.names().contains(&"Inbox".to_owned())
                && places.names().contains(&"Receipts".to_owned()),
            "mailboxes and folders are listed: {:?}",
            places.names()
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!places.is_open(), "Escape closed the popover");
        support::keys(&window, &["g", "o"]);
        assert!(places.is_open(), "g o opened it again");
        places.set_filter("trav");
        assert_eq!(places.names(), ["Travel"], "typing filters the places");
        support::click_row_saying(&window, &window, "Travel");
        assert!(
            crate::settle_until(async || !places.is_open()).await,
            "clicking a place closed the popover"
        );

        assert!(
            crate::settle_until(async || {
                support::subjects(&window) == ["Hotel booking", "Train to the coast"]
            })
            .await,
            "Travel's conversations are not shown: {:?}",
            support::subjects(&window)
        );
        assert_eq!(window.place_name(), "Travel", "the header names the place");

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "g i did not come back to the inbox: {:?}",
            support::subjects(&window)
        );
        assert_eq!(window.place_name(), "Inbox");
    });
}

/// The popover lists Flagged (`g *`) and Snoozed (`g z`) among the
/// mailboxes, each with its key inside the row, and choosing one lists the
/// same cross-account view the key does.
pub fn flagged_and_snoozed_are_listed_and_open_their_views() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
            .await;
        let (harbor, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor draft", "x", 20)
            .await;
        fixture
            .file(
                ("Tomas Reyes", "tomas@example.net"),
                "Staffing plan",
                "x",
                10,
            )
            .await;
        crate::commands::snooze(&fixture, harbor).await;
        let (window, client) = fixture.open().await;
        crate::commands::flag(&client, atlas).await;

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        for (name, key) in [("Flagged", "g *"), ("Snoozed", "g z")] {
            assert!(
                crate::settle_until(async || places.names().contains(&name.to_owned())).await,
                "the popover never listed {name}: {:?}",
                places.names()
            );
            let said = row_saying(&window, name).await;
            assert!(
                said.iter().any(|text| text == key),
                "the {name} row does not show {key}: {said:?}"
            );
        }

        support::click_row_saying(&window, &window, "Flagged");
        assert!(
            crate::settle_until(async || {
                window.place_name() == "Flagged" && support::subjects(&window) == ["Atlas budget"]
            })
            .await,
            "choosing Flagged listed {:?} under {:?}",
            support::subjects(&window),
            window.place_name()
        );
        support::keys(&window, &["g", "o"]);
        support::click_row_saying(&window, &window, "Snoozed");
        assert!(
            crate::settle_until(async || {
                window.place_name() == "Snoozed" && support::subjects(&window) == ["Harbor draft"]
            })
            .await,
            "choosing Snoozed listed {:?} under {:?}",
            support::subjects(&window),
            window.place_name()
        );
    });
}

/// Every mailbox the popover lists shows the key that goes there, `g #`
/// takes the Trash by that key, the line under the places follows the
/// highlighted row, and the Inbox's count is the strip's.
pub fn every_mailbox_shows_its_key_and_the_footer_follows_the_highlight() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Inbox mail", "Hello.", 5)
            .await;
        let junk = fixture.folder("Junk").await;
        fixture.file_in(junk, "Old news", 60).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "the inbox never reached the screen"
        );

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Junk".to_owned())).await,
            "the popover never listed Junk: {:?}",
            places.names()
        );
        let said = row_saying(&window, "Junk").await;
        assert!(
            said.iter().any(|text| text == "g j"),
            "the Trash row shows no key: {said:?}"
        );
        // The Inbox's count is the conversations the strip counts.
        let strip = support::texts(&window)
            .into_iter()
            .find(|text| text.contains(" unread") && text.contains('\u{b7}'))
            .expect("the strip counts the inbox");
        let inbox = row_saying(&window, "Inbox").await;
        let count = strip.split_whitespace().next().unwrap_or_default();
        assert!(
            inbox.iter().any(|text| text == count),
            "the Inbox row says {inbox:?}, the strip {strip:?}"
        );
        // The footer says what Return does on the highlighted row.
        assert!(
            places.footer().contains("in:Inbox"),
            "the first row is the Inbox: {:?}",
            places.footer()
        );
        support::press(&window, "Down", gtk::gdk::ModifierType::empty());
        assert!(
            !places.footer().contains("in:Inbox"),
            "the footer did not move with the highlight: {:?}",
            places.footer()
        );

        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        support::keys(&window, &["g", "j"]);
        assert!(
            crate::settle_until(async || {
                window.place_name() == "Junk" && support::subjects(&window) == ["Old news"]
            })
            .await,
            "g j listed {:?} under {:?}",
            support::subjects(&window),
            window.place_name()
        );
    });
}

/// Every place says how many conversations it holds, the same number its own
/// view shows: Flagged and Snoozed, which are views over mail filed
/// elsewhere, and each label, as the mailboxes are.
pub fn every_place_shows_its_count() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Atlas budget", "x", 30)
            .await;
        let (harbor, _) = fixture
            .file(("Lena Park", "lena@example.org"), "Harbor draft", "x", 20)
            .await;
        let (staffing, _) = fixture
            .file(
                ("Tomas Reyes", "tomas@example.net"),
                "Staffing plan",
                "x",
                10,
            )
            .await;
        fixture.label(atlas, &["Atlas"]).await;
        fixture.label(harbor, &["Harbor"]).await;
        crate::commands::snooze(&fixture, staffing).await;
        let (window, client) = fixture.open().await;
        crate::commands::flag(&client, atlas).await;
        crate::commands::flag(&client, harbor).await;

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Harbor".to_owned())).await,
            "the popover never listed the labels: {:?}",
            places.names()
        );
        for (name, count) in [
            ("Flagged", "2"),
            ("Snoozed", "1"),
            ("Atlas", "1"),
            ("Harbor", "1"),
        ] {
            let said = row_saying(&window, name).await;
            let counted = crate::settle_until(async || {
                row_saying(&window, name)
                    .await
                    .iter()
                    .any(|text| text == count)
            })
            .await;
            assert!(
                counted,
                "the {name} row should say {count}, as its view lists: {said:?}"
            );
        }
    });
}

/// What the popover's row for `name` says, key included.
async fn row_saying(window: &postio_gtk::window::FocusWindow, name: &str) -> Vec<String> {
    use gtk::prelude::*;
    let said = std::cell::RefCell::new(Vec::new());
    crate::settle_until(async || {
        let found = support::descendants(window)
            .into_iter()
            .find(|widget| {
                widget.is::<gtk::ListBoxRow>()
                    && widget.is_mapped()
                    && support::texts(widget).iter().any(|text| text == name)
            })
            .map(|row| support::texts(&row))
            .unwrap_or_default();
        let done = !found.is_empty();
        said.replace(found);
        done
    })
    .await;
    said.into_inner()
}
