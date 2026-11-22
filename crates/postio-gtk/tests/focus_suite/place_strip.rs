//! A place that is not the inbox describes itself: its strip counts what it lists, the has-action
//! toggle goes with the inbox it narrows, and an empty place says why and the way out instead of a
//! blank page.

use gtk::prelude::*;
use postio_core::CommandId;

use crate::support::{self, only, texts, with_class};

fn empty_says(window: &postio_gtk::window::FocusWindow) -> Vec<String> {
    with_class(window, "focus-empty")
        .into_iter()
        .find(|page| page.is_mapped())
        .map(|page| support::texts(&page))
        .unwrap_or_default()
}

pub fn an_empty_snoozed_place_says_why_and_the_way_back() {
    crate::gtk_case(async {
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
