//! The cursor and the selection are two things (constitution II; FR-015):
//! the cursor is where the keyboard is, the selection is what `a` would
//! archive, and the bulk bar says what is selected and what to do with it
//! (T045; US1 scenario 3).

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};

use crate::support::{self, Fixture, only, texts};

pub fn three_selected_and_the_cursor_on_a_fourth_archives_exactly_the_three() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;

        // x on the first three, stepping with j, and j once more: the
        // cursor sits on the fourth, which is not selected.
        support::keys(&window, &["j", "x", "j", "x", "j", "x", "j"]);
        let bar = only(&window, "focus-bulk-bar");
        assert!(
            bar.is_mapped(),
            "the bulk bar shows while anything is selected"
        );
        let said = texts(&bar);
        assert!(
            said.iter().any(|text| text == "3 selected"),
            "the bar counts the selection: {said:?}"
        );
        for command in [CommandId::Archive, CommandId::Snooze, CommandId::ToggleRead] {
            let key = postio_ui::hints::key(Keymap::defaults(), command).expect("a key");
            assert!(
                said.contains(&key),
                "the bar shows {command}'s key {key:?}: {said:?}"
            );
        }

        support::keys(&window, &["a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Fourth", "Fifth"]).await,
            "exactly the three selected left the inbox, and the cursor's row did not: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || !bar.is_mapped()).await,
            "an archive leaves nothing selected, so the bar goes"
        );
    });
}

pub fn escape_clears_the_selection_and_the_cursor_stays() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j", "x", "J", "J"]);
        let bar = only(&window, "focus-bulk-bar");
        assert!(
            texts(&bar).iter().any(|text| text == "3 selected"),
            "J extends the selection with the cursor: {:?}",
            texts(&bar)
        );
        let cursor = window.pane().expect("the inbox").cursor().selected();
        assert_eq!(cursor, 2, "the cursor went down with the extension");
        support::keys(&window, &["Escape"]);
        assert!(!bar.is_mapped(), "Escape clears the selection");
        assert_eq!(
            window.pane().expect("the inbox").cursor().selected(),
            cursor,
            "and leaves the cursor where it was"
        );
        support::keys(&window, &["X"]);
        assert!(
            window.selection().is_everything(),
            "X selects the whole view as a predicate, naming no row (FR-015)"
        );
        assert!(
            texts(&bar).iter().any(|text| text == "5 selected"),
            "counted from the list's total, not by walking it: {:?}",
            texts(&bar)
        );
    });
}
