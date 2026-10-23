//! US1 scenario 4: j and k move only the cursor -- nothing opens, nothing
//! is marked read, nothing is selected (FR-016). Selecting or moving to a
//! row is never reading it.

use crate::support::{self, Fixture};

pub fn j_and_k_move_only_the_cursor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        let before = client.counts().snapshot();

        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "the list opens with the cursor on the first row"
        );
        support::keys(&window, &["j"]);
        assert_eq!(
            pane.cursor().selected(),
            1,
            "j moves the cursor to the second row"
        );
        support::keys(&window, &["j", "j"]);
        assert_eq!(pane.cursor().selected(), 3, "and walks it down");
        support::keys(&window, &["k"]);
        assert_eq!(pane.cursor().selected(), 2, "k walks it back up");

        // Nothing else: no command reached the host, no body was read, the
        // selection is empty, and the rows the cursor passed are unread.
        crate::settle();
        let after = client.counts().snapshot();
        for family in ["Send", "SendTracked", "Body", "Readings", "ThreadReadings"] {
            assert_eq!(
                after.get(family),
                before.get(family),
                "moving the cursor asked the host for {family}: {after:?}"
            );
        }
        assert!(window.selection().is_empty(), "nothing was selected");
        assert!(
            crate::settle_until(async || {
                pane.rows_on_screen()
                    .iter()
                    .all(|row| !row.drawn().texts.is_empty())
            })
            .await,
            "the rows on screen were never drawn"
        );
        for row in pane.rows_on_screen() {
            assert!(
                row.drawn().bold,
                "{:?} was marked read by the cursor passing over it",
                row.drawn().texts
            );
        }
    });
}

/// Every list opens with the cursor on its first row, and opening it opens
/// nothing and marks nothing read (maintainer, 2026-10-07).
pub fn a_list_opens_with_the_cursor_on_its_first_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        let before = client.counts().snapshot();

        // At launch, before any key.
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "the window opened with the cursor on {}, not the first row",
            pane.cursor().selected()
        );
        assert!(
            window.reading().is_none(),
            "the cursor on the first row opened it"
        );

        // `j` moves to the second row, not onto the first.
        support::keys(&window, &["j"]);
        assert_eq!(pane.cursor().selected(), 1, "j walks on from the first row");

        // Going to the inbox again replaces the list's rows: the cursor
        // is back on the first.
        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "g i left the cursor on {}",
            pane.cursor().selected()
        );

        // Back from a page that replaced the list.
        support::keys(&window, &["j", "j"]);
        support::keys(&window, &["g", "f"]);
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "g f never opened Filtered"
        );
        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || window.filtered().is_none()).await,
            "g i never left Filtered"
        );
        assert_eq!(
            pane.cursor().selected(),
            0,
            "coming back from Filtered left the cursor off the first row"
        );

        crate::settle();
        let after = client.counts().snapshot();
        for family in ["Send", "SendTracked", "Body", "Readings", "ThreadReadings"] {
            assert_eq!(
                after.get(family),
                before.get(family),
                "putting the cursor on the first row asked the host for {family}"
            );
        }
        assert!(
            crate::settle_until(async || {
                pane.rows_on_screen()
                    .iter()
                    .all(|row| !row.drawn().texts.is_empty())
            })
            .await,
            "the rows on screen were never drawn"
        );
        for row in pane.rows_on_screen() {
            assert!(row.drawn().bold, "{:?} was marked read", row.drawn().texts);
        }
    });
}

/// GTK's list view moves the cursor on its own -- a click on a row, a row
/// archived from under it -- and the controller walks on from wherever it
/// is, not from where it last put it itself.
pub fn the_list_view_moving_the_cursor_is_where_j_walks_on_from() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "the list opens with the cursor on the first row"
        );

        pane.cursor().set_selected(2);
        support::keys(&window, &["j"]);
        assert_eq!(
            pane.cursor().selected(),
            3,
            "j walks on from the row the list view chose"
        );
        pane.cursor().set_selected(1);
        support::keys(&window, &["k"]);
        assert_eq!(pane.cursor().selected(), 0, "and k back from it");
        assert!(window.selection().is_empty(), "moving selected nothing");
    });
}

/// `g i` and then `j j`, faster than the store answers: the rows on screen
/// are the ones the keys walked, and the list that lands a beat later leaves
/// the cursor on the row it reached rather than walking it back to the top.
pub fn keys_before_a_reopened_list_lands_are_not_walked_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        let pane = window.pane().expect("the inbox");
        assert!(
            crate::settle_until(async || {
                pane.feed().has_landed() && pane.cursor().selected() == 0
            })
            .await,
            "the inbox never opened on its first row"
        );

        window.act(postio_core::CommandId::GoToInbox);
        assert!(
            !pane.feed().has_landed(),
            "the reopened inbox landed before a key could be pressed"
        );
        support::keys(&window, &["j", "j"]);
        assert_eq!(pane.cursor().selected(), 2, "j walks the rows on screen");

        // The positive control: the page did land, after the keys.
        assert!(
            crate::settle_until(async || pane.feed().has_landed()).await,
            "the reopened inbox never landed"
        );
        crate::settle();
        assert_eq!(
            pane.cursor().selected(),
            2,
            "the list landing walked the cursor back to the top"
        );
    });
}
