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

/// A folder chosen in the popover opens with the cursor on its first row.
pub fn a_folder_from_the_popover_opens_with_the_cursor_on_its_first_row() {
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
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Inbox mail"]).await,
            "the inbox never reached the screen"
        );
        support::keys(&window, &["g", "o"]);
        let places = window.places().expect("g o opened the folders popover");
        assert!(
            crate::settle_until(async || places.names().contains(&"Travel".to_owned())).await,
            "the popover never listed Travel"
        );
        places.set_filter("trav");
        support::click_row_saying(&window, &window, "Travel");
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "Travel's conversations are not shown: {:?}",
            support::subjects(&window)
        );
        let pane = window.pane().expect("the list");
        assert_eq!(
            pane.cursor().selected(),
            0,
            "Travel opened with the cursor off its first row"
        );
    });
}
