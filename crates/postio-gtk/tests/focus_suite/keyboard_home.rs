//! The keyboard has one home: the list. At launch, after Escape from the
//! bar, after going to a place and after an overlay closes, the window's
//! real keyboard focus is in the list -- never on the top bar's Compose
//! button or the header's place button, where Space or Return would press a
//! control instead of acting on the row (storyboard review, 2026-10).

use gtk::prelude::*;

use crate::support;

/// Whether the window's keyboard focus is the list: its view, or a row in it.
fn in_the_list(window: &postio_gtk::window::FocusWindow) -> bool {
    let view = window.pane().expect("the inbox").view().clone();
    gtk::prelude::GtkWindowExt::focus(window)
        .is_some_and(|focus| focus == view || focus.is_ancestor(&view))
}

fn said(window: &postio_gtk::window::FocusWindow) -> String {
    format!("the keyboard is on {}", support::focus_path(window))
}

pub fn the_window_opens_with_the_keyboard_in_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        crate::settle();
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn escape_from_the_bar_returns_the_keyboard_to_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::press(&window, "slash", gtk::gdk::ModifierType::empty());
        let bar = window.bar().expect("/ opened the bar");
        assert!(bar.is_open(), "the bar is up");
        assert!(!in_the_list(&window), "the keyboard is in the bar's entry");
        support::press(&window, "Escape", gtk::gdk::ModifierType::empty());
        assert!(!bar.is_open(), "Escape closed the bar");
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn going_to_a_place_leaves_the_keyboard_in_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["g", "l"]);
        let places = window.places().expect("g l opened the folders popover");
        assert!(places.is_open(), "the popover is up");
        places.activate();
        assert!(
            crate::settle_until(async || !places.is_open()).await,
            "choosing a place closed the popover"
        );
        crate::settle();
        assert!(in_the_list(&window), "{}", said(&window));

        window.act(postio_core::CommandId::GoToFlagged);
        crate::settle();
        assert_eq!(window.place_name(), "Flagged");
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

pub fn the_keyboard_follows_the_cursor_row() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j"]);
        crate::settle();
        let pane = window.pane().expect("the inbox");
        assert_eq!(pane.cursor().selected(), 1);
        assert!(in_the_list(&window), "{}", said(&window));
    });
}

/// A list that opens puts the cursor on its first row, and the keyboard
/// goes with it when it was on a row: Space or Return then acts on the row
/// the ring is drawn on, not on a row the list used to hold.
pub fn a_list_that_opens_takes_the_keyboard_from_a_row_to_the_first() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let pane = window.pane().expect("the inbox");
        support::keys(&window, &["j", "j"]);
        pane.focus_cursor();
        crate::settle();
        assert_eq!(pane.cursor().selected(), 2);

        support::keys(&window, &["g", "i"]);
        assert!(
            crate::settle_until(async || pane.cursor().selected() == 0).await,
            "g i left the cursor on {}",
            pane.cursor().selected()
        );
        // The list view rebuilds its rows on the next layout, and the
        // row that held the keyboard goes with the old ones: for a turn or
        // more the window has no focus at all, then scroll_to(FOCUS) puts it
        // on the new first row. Wait for that, not for one turn of the loop.
        let on_the_first_row = || {
            let first = pane.rows_on_screen().into_iter().next()?;
            let item = first.ancestor(gtk::glib::Type::from_name("GtkListItemWidget")?)?;
            let focus = gtk::prelude::GtkWindowExt::focus(&window)?;
            Some(focus == item || focus.is_ancestor(&item))
        };
        assert!(
            crate::settle_until(async || on_the_first_row() == Some(true)).await,
            "the keyboard stayed on a row the cursor left: {}",
            support::focus_path(&window)
        );
    });
}
