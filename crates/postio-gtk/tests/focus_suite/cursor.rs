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

        support::keys(&window, &["j"]);
        assert_eq!(
            pane.cursor().selected(),
            0,
            "j puts the cursor on the first row"
        );
        support::keys(&window, &["j", "j"]);
        assert_eq!(pane.cursor().selected(), 2, "and walks it down");
        support::keys(&window, &["k"]);
        assert_eq!(pane.cursor().selected(), 1, "k walks it back up");

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
