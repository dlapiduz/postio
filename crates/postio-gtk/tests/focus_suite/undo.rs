//! US1 scenario 3's second half: the toast says what happened, and one
//! Ctrl+Z returns all three -- including after the toast has gone, because
//! the toast going is not the undo window closing (FR-041).

use gtk::prelude::*;

use crate::support::{self, Fixture};

pub fn one_ctrl_z_returns_all_three_after_the_toast_has_gone() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (window, _client) = fixture.five().await;
        support::keys(&window, &["j", "x", "j", "x", "j", "x", "j", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window) == ["Fourth", "Fifth"]).await,
            "the three never left: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || {
                window.toast_showing().as_deref() == Some("Archived 3 messages")
            })
            .await,
            "the toast says what happened: {:?}",
            window.toast_showing()
        );

        // The toast goes -- it would on its own after eight seconds -- and
        // the undo does not go with it.
        window.dismiss_toast();
        crate::settle();
        assert_eq!(window.toast_showing(), None);

        support::press(&window, "z", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 5).await,
            "one Ctrl+Z returns all three: {:?}",
            support::subjects(&window)
        );
        assert_eq!(
            support::subjects(&window),
            ["First", "Second", "Third", "Fourth", "Fifth"],
            "each back where it was"
        );
    });
}

/// What the toast on screen says and whether it still offers Undo.
fn toast_of(window: &postio_gtk::window::FocusWindow) -> (Option<String>, bool) {
    let toast = window.toast();
    (
        window.toast_showing(),
        toast.is_some_and(|toast| toast.button_label().is_some()),
    )
}

/// The toast counts what the person archived -- rows, not the messages
/// inside them -- and an undo says it was an undo, not that the mail was
/// archived. Frame by frame (2026-10 review): a thread of three archived with
/// `a` said "Archived 3 messages" for the one row the person chose, and the
/// toast after `u` repeated "Archived …" with its button gone.
pub fn the_toast_counts_the_row_and_says_when_it_was_undone() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture.thread_of("Plans", 3, 10).await;
        fixture
            .file(
                ("Lena Park", "lena@example.org"),
                "Harbor draft",
                "Version.",
                30,
            )
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the two conversations never reached the screen: {:?}",
            support::subjects(&window)
        );

        support::keys(&window, &["j", "a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 1).await,
            "the thread did not leave: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || {
                toast_of(&window) == (Some("Archived 1 message".to_owned()), true)
            })
            .await,
            "one row archived, whatever it holds: {:?}",
            toast_of(&window)
        );

        support::press(&window, "z", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the thread did not come back: {:?}",
            support::subjects(&window)
        );
        assert!(
            crate::settle_until(async || {
                toast_of(&window) == (Some("Archived 1 message, undone".to_owned()), false)
            })
            .await,
            "the toast after an undo says it was undone, with nothing to undo: {:?}",
            toast_of(&window)
        );
    });
}
