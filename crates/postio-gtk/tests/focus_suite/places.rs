use crate::support::{self, Fixture};

pub fn the_folders_popover_quiets_the_lists_ring() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Inbox mail", "Hello.", 5).await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "the inbox never reached the screen"
        );
        let ring = || {
            window
                .observe()
                .app
                .get("focus.cursor.ring")
                .and_then(|ring| ring.as_str().map(str::to_owned))
        };
        assert_eq!(
            ring().as_deref(),
            Some("accent"),
            "the list has the keyboard"
        );
        support::keys(&window, &["g", "l"]);
        assert!(window.places().is_some(), "g l opened the folders popover");
        assert!(
            crate::settle_until(async || ring().as_deref() == Some("quiet")).await,
            "the list kept its ring under the popover: {:?}",
            ring()
        );
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || ring().as_deref() == Some("accent")).await,
            "the ring did not come back: {:?}",
            ring()
        );
    });
}

pub fn flagged_and_snoozed_are_listed_and_open_their_views() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture.file_from_ada("Atlas budget", "x", 30).await;
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

        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
        for (name, key) in [("Flagged", "g s"), ("Snoozed", "g b")] {
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
        support::keys(&window, &["g", "l"]);
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

pub fn every_place_shows_its_count() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let (atlas, _) = fixture.file_from_ada("Atlas budget", "x", 30).await;
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

        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
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
