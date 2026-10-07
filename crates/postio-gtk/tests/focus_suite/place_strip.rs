//! A place that is not the inbox describes itself: the strip beside its name
//! counts what it lists (the inbox's "59 · 21 unread" described a place the
//! person had left), the has-action toggle goes with the inbox it narrows,
//! and a place with nothing in it says why and the way out instead of a
//! blank page (ux-architect: nothing is a dead end).

use gtk::prelude::*;
use postio_core::CommandId;

use crate::support::{self, Fixture, only, texts, with_class};

/// What the empty page says, when it is the page showing.
fn empty_says(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    with_class(window, "focus-empty")
        .into_iter()
        .find(|page| page.is_mapped())
        .map(|page| support::texts(&page))
        .unwrap_or_default()
}

pub fn flagged_counts_what_is_flagged_not_the_inbox() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        assert!(
            crate::settle_until(async || {
                texts(&only(&window, "focus-counts"))
                    .iter()
                    .any(|said| said == "5 \u{b7} 5 unread")
            })
            .await,
            "the inbox counts its five: {:?}",
            texts(&only(&window, "focus-counts"))
        );

        support::keys(&window, &["j"]);
        window.act(CommandId::Flag);
        crate::settle();
        support::keys(&window, &["j"]);
        window.act(CommandId::Flag);
        crate::settle();

        window.act(CommandId::GoToFlagged);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "Flagged lists the two: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || { texts(&only(&window, "focus-counts")) == ["2"] }).await,
            "the strip counts what Flagged lists, with no unread of the inbox's: {:?}",
            texts(&only(&window, "focus-counts"))
        );
        assert!(
            !only(&window, "focus-has-action").is_visible(),
            "the has-action toggle narrows the inbox and is not offered here"
        );

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || {
                texts(&only(&window, "focus-counts"))
                    .iter()
                    .any(|said| said.contains("unread"))
            })
            .await,
            "back in the inbox the strip counts it again: {:?}",
            texts(&only(&window, "focus-counts"))
        );
        assert!(
            only(&window, "focus-has-action").is_visible(),
            "and the toggle is back"
        );
    });
}

pub fn an_empty_snoozed_place_says_why_and_the_way_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        window.act(CommandId::GoToSnoozed);
        assert!(
            crate::settle_until(async || !empty_says(&window).is_empty()).await,
            "an empty Snoozed is not a blank pane"
        );
        let said = empty_says(&window);
        assert!(said.contains(&"Nothing snoozed".to_owned()), "{said:?}");
        assert!(
            said.iter().any(|line| line.contains("snooze")),
            "it says how a message comes to be here: {said:?}"
        );
        assert!(
            said.contains(&"back to the inbox".to_owned()),
            "and names the way back: {said:?}"
        );
        assert_eq!(
            texts(&only(&window, "focus-counts")),
            ["0"],
            "the strip counts the nothing it lists"
        );

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || empty_says(&window).is_empty()).await,
            "going back to the inbox puts the list back"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "and the three conversations are listed: {:?}",
            support::subjects(&window)
        );
    });
}
