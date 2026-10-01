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
        let travel = support::row_saying(&window, "Travel");
        support::click(&window, &travel, 1);
        assert!(!places.is_open(), "Enter closed the popover");

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
