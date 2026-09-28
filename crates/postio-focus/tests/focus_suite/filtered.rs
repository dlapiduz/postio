//! The Filtered view (US9, T124; screen 21): `g f` lists what filing
//! archived, newest first, each with its reason; `1`-`7` narrow it to a
//! reason; `R` restores the focused row, and `Ctrl+Z` takes that back.

use gtk::gdk;

use crate::support::{self, Fixture};

/// Four filtered messages and one conversation in the inbox.
async fn fixture() -> Fixture {
    let fixture = Fixture::empty().await;
    fixture
        .file(
            ("Ada Moreno", "ada@example.com"),
            "Atlas budget",
            "Numbers.",
            5,
        )
        .await;
    for (from, subject, reason, source, minutes) in [
        (
            ("Forge", "noreply@forge.test"),
            "Review requested",
            "notification",
            Some("Forge"),
            10,
        ),
        (
            ("Outdoor Supply", "deals@outdoor.test"),
            "30% off tents",
            "promotion",
            None,
            20,
        ),
        (
            ("Lucky Rewards", "win@lucky.test"),
            "You have been selected",
            "spam",
            None,
            30,
        ),
        // Forty days ago: Filtered never deletes (scenario 7).
        (
            ("Forge", "noreply@forge.test"),
            "Build 1182 passed",
            "notification",
            Some("Forge"),
            40 * 24 * 60,
        ),
    ] {
        fixture
            .filtered(from, subject, reason, source, minutes)
            .await;
    }
    fixture
}

/// The Filtered view's row subjects, top to bottom.
fn listed(window: &postio_focus::window::FocusWindow) -> Vec<String> {
    window
        .filtered()
        .map(|view| view.subjects())
        .unwrap_or_default()
}

/// US9 scenarios 5 and 7: every filtered message, newest first, the old
/// one included; the Notifications tab's number lists only notifications,
/// one request to the store.
pub fn g_f_lists_filtered_mail_and_its_number_keys_narrow_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, client) = fixture.open().await;
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || {
                listed(&window)
                    == [
                        "Review requested",
                        "30% off tents",
                        "You have been selected",
                        "Build 1182 passed",
                    ]
            })
            .await,
            "g f did not list the filtered mail, newest first: {:?}",
            listed(&window)
        );
        let view = window.filtered().expect("the view");
        let said = view.texts();
        for wanted in [
            "Filtered",
            "All",
            "4",
            "Notifications",
            "2",
            "notification \u{b7} Forge",
        ] {
            assert!(
                said.iter().any(|line| line == wanted),
                "no {wanted:?} in {said:?}"
            );
        }

        let before = client.counts().snapshot();
        support::press(&window, "4", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || {
                listed(&window) == ["Review requested", "Build 1182 passed"]
            })
            .await,
            "the Notifications tab did not narrow the list: {:?}",
            listed(&window)
        );
        let after = client.counts().snapshot();
        let asked: Vec<(&str, u64)> = after
            .iter()
            .map(|(family, count)| (*family, count - before.get(family).copied().unwrap_or(0)))
            .filter(|(_, count)| *count > 0)
            .collect();
        assert_eq!(asked, [("Filtered", 1)], "a tab is one page read");

        support::press(&window, "Escape", gdk::ModifierType::empty());
        assert!(
            crate::settle_until(async || window.filtered().is_none()).await,
            "Escape went back to the inbox"
        );
    });
}

/// US9 scenario 4, in the view: `R` restores the focused row -- it leaves
/// Filtered for the inbox -- and one `Ctrl+Z` takes it back.
pub fn r_restores_the_focused_row_and_ctrl_z_takes_it_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = fixture().await;
        let (window, _client) = fixture.open().await;
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "Filtered never listed: {:?}",
            listed(&window)
        );
        let view = window.filtered().expect("the view");
        assert!(
            view.texts()
                .iter()
                .any(|line| line == "Restore, never filter this sender"),
            "the focused row offers its restore: {:?}",
            view.texts()
        );
        support::press(&window, "R", gdk::ModifierType::SHIFT_MASK);
        assert!(
            crate::settle_until(async || {
                listed(&window)
                    == [
                        "30% off tents",
                        "You have been selected",
                        "Build 1182 passed",
                    ]
            })
            .await,
            "R did not take the row out of Filtered: {:?}",
            listed(&window)
        );
        support::press(&window, "z", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || listed(&window).len() == 4).await,
            "Ctrl+Z did not put it back: {:?}",
            listed(&window)
        );
    });
}

/// T126: the header strip says how many were filtered since local
/// midnight, with `g f`, and pressing it opens Filtered; the folders
/// popover lists Filtered with the same count.
pub fn the_strip_counts_what_was_filtered_today() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(
                ("Ada Moreno", "ada@example.com"),
                "Atlas budget",
                "Numbers.",
                5,
            )
            .await;
        // Filed a minute and two ago, by the real clock: today. And one
        // forty days ago, which is not.
        let ago = |minutes: i64| (support::now() - chrono::Utc::now()).num_minutes() + minutes;
        fixture
            .filtered(
                ("Forge", "noreply@forge.test"),
                "Review requested",
                "notification",
                Some("Forge"),
                ago(1),
            )
            .await;
        fixture
            .filtered(
                ("Outdoor Supply", "deals@outdoor.test"),
                "30% off tents",
                "promotion",
                None,
                ago(2),
            )
            .await;
        fixture
            .filtered(
                ("Forge", "noreply@forge.test"),
                "Old build",
                "notification",
                Some("Forge"),
                40 * 24 * 60,
            )
            .await;
        let (window, _client) = fixture.open().await;
        let chrome = window.chrome().expect("the strip");
        assert!(
            crate::settle_until(async || {
                chrome.filtered_today_said().as_deref() == Some("2 filtered today")
            })
            .await,
            "the strip does not count today's: {:?}",
            chrome.filtered_today_said()
        );
        let said = support::texts(chrome.strip());
        assert!(
            said.iter().any(|text| text == "g f"),
            "with its key: {said:?}"
        );

        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Filtered".to_owned())).await,
            "the popover lists no Filtered: {:?}",
            places.names()
        );
        places.set_filter("filt");
        places.activate();
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "the popover's Filtered row did not open Filtered"
        );
        support::press(&window, "Escape", gdk::ModifierType::empty());
        chrome.press_filtered_today();
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "pressing the strip's count did not open Filtered"
        );
    });
}
