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

/// `a a` works down the list (#1746): the row below the archived one takes
/// the cursor, so the second `a` archives it rather than nothing.
pub fn archive_hands_the_cursor_to_the_row_below() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j", "j"]);
        let cursor = || window.pane().expect("the inbox").cursor().selected();
        assert_eq!(cursor(), 1, "two j put the cursor on the second row");

        support::keys(&window, &["a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 4).await,
            "a archived the cursor's row: {:?}",
            support::subjects(&window)
        );
        assert_eq!(
            cursor(),
            1,
            "the row below slid into the archived one's place and holds the cursor"
        );

        support::keys(&window, &["a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the second a archived the row that took the cursor: {:?}",
            support::subjects(&window)
        );
        assert_eq!(cursor(), 1, "and the cursor is still on the second row");
        assert_eq!(
            support::subjects(&window)[0],
            "First",
            "the row above the cursor was never touched"
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

pub fn a_select_all_archives_what_focus_lists_and_never_held_mail() {
    // `X` is a predicate over the view (FR-015), so which view it names is
    // the whole question: Focus's inbox leaves out held digest mail, and the
    // unified inbox does not. `a` after `X` must take exactly what the list
    // shows (T167).
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (held, _) = fixture
            .file(
                ("Weir Level", "levels@example.test"),
                "Held for the digest",
                "Held.",
                1,
            )
            .await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::repository::DigestRepository::new(&connection)
                .hold(held, "levels", support::now())
                .await
                .expect("held");
        }
        let (window, _client) = fixture.five().await;

        support::keys(&window, &["X", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).is_empty()).await,
            "a select-all archives every conversation the list shows: {:?}",
            support::subjects(&window)
        );
        let connection = fixture.database.connect().await.expect("a connection");
        let after = postio_storage::repository::MessageRepository::new(&connection)
            .get(held)
            .await
            .expect("a read")
            .expect("the held message");
        assert_eq!(
            after.mailbox_id, fixture.inbox,
            "the held message was archived too: a select-all in Focus reached \
             past what Focus lists"
        );
    });
}

/// The button named `class` under `root`, as a person would press it.
fn button(root: &impl IsA<gtk::Widget>, class: &str) -> gtk::Button {
    only(root, class)
        .downcast::<gtk::Button>()
        .expect("a button")
}

/// T179: Delete has a button with its key on the bulk bar, and pressing it
/// deletes what is selected, as the key does.
pub fn the_bulk_bar_has_a_delete_button_that_deletes_the_selection() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j", "x", "j", "x", "j"]);
        let bar = only(&window, "focus-bulk-bar");
        let delete = button(&bar, "focus-bulk-delete");
        let key = postio_ui::hints::key(Keymap::defaults(), CommandId::Delete).expect("a key");
        assert!(
            texts(&delete).contains(&key),
            "the button shows Delete's key {key:?}: {:?}",
            texts(&delete)
        );
        assert!(
            texts(&delete).iter().any(|text| text == "Delete"),
            "the button says what it does: {:?}",
            texts(&delete)
        );
        support::click(&window, &delete, 1);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Third", "Fourth", "Fifth"])
                .await,
            "the two selected conversations left the inbox: {:?}",
            support::subjects(&window)
        );
    });
}
