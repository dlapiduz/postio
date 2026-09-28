//! US1 scenario 5: `!` narrows the inbox to the rows with a marker (FR-017).
//! The toggle carries the count, the strip says how many of how many are
//! showing, the selection is cleared, and the cursor stays on the same
//! message while it is still shown. `!` again restores the inbox.

use crate::support::{self, Fixture, only, texts};

pub fn has_action_narrows_to_the_marked_rows_and_back() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let filed = fixture.file_five().await;
        // Second and Fourth ask something.
        fixture.ask(filed[1], "Can you look at this?").await;
        fixture.ask(filed[3], "Could you sign it by Friday?").await;
        let (window, client) = fixture.open_five().await;
        assert!(
            crate::settle_until(async || {
                texts(&only(&window, "focus-has-action"))
                    .iter()
                    .any(|said| said == "Has action \u{b7} 2")
            })
            .await,
            "the toggle carries the count: {:?}",
            texts(&only(&window, "focus-has-action"))
        );
        assert!(
            texts(&only(&window, "focus-counts"))
                .iter()
                .any(|said| said == "5 \u{b7} 5 unread"),
            "the strip counts the conversations and the unread: {:?}",
            texts(&only(&window, "focus-counts"))
        );

        // First selected; the cursor on Fourth, which is marked.
        support::keys(&window, &["j", "x", "j", "j", "j"]);
        assert!(!window.selection().is_empty());

        support::keys(&window, &["exclam"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Second", "Fourth"]).await,
            "only the marked rows remain: {:?}",
            support::subjects(&window)
        );
        let strip = texts(&only(&window, "focus-header-strip"));
        assert!(
            strip
                .iter()
                .any(|said| said == "Showing 2 of 5 \u{b7} ! again to show all"),
            "the strip says how many of how many: {strip:?}"
        );
        assert!(
            window.selection().is_empty(),
            "the filter clears the selection"
        );
        let cursor = window.cursor_row().map(|row| {
            let postio_focus::list::FocusRow::Conversation(row) = row;
            row.summary.representative.subject.unwrap_or_default()
        });
        assert_eq!(
            cursor.as_deref(),
            Some("Fourth"),
            "the cursor stays on the same message, which is still shown"
        );

        support::keys(&window, &["exclam"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 5).await,
            "! again restores the inbox: {:?}",
            support::subjects(&window)
        );
        assert!(
            client.counts().of("FocusCounts") >= 1,
            "the counts are the host's, read as one request: {:?}",
            client.counts().snapshot()
        );
    });
}
