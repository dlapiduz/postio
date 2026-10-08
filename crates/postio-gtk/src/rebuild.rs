//! Redrawing a list without losing the keyboard (T200).
//!
//! A list that clears its rows and builds them again destroys the row that
//! holds the keyboard, and the window is left with a focus widget that has no
//! parents. GTK delivers a key from the focus up, so none reaches the window
//! until a click: `j`, `Delete` or `Esc` do nothing, about once in two runs,
//! whenever an asynchronous read lands after the list opened. This is the one
//! place that says what happens to the keyboard when rows are rebuilt.

use gtk::prelude::*;

/// Run `rebuild`, which clears and refills `list`, and keep the keyboard
/// where it was: on the row that stands at the place of the one that had it,
/// or the nearest one that can take it, or the window when the list is empty.
/// Does nothing about the keyboard when it was not in the list.
pub(crate) fn keeping_focus(list: &gtk::ListBox, rebuild: impl FnOnce()) {
    let focused_at = list
        .root()
        .and_then(|root| root.focus())
        .filter(|focus| focus == list.upcast_ref::<gtk::Widget>() || focus.is_ancestor(list))
        .and_then(|focus| {
            let mut widget = Some(focus);
            while let Some(current) = widget {
                if let Some(row) = current.downcast_ref::<gtk::ListBoxRow>() {
                    return Some(row.index());
                }
                widget = current.parent();
            }
            None
        });
    rebuild();
    let Some(at) = focused_at else {
        return;
    };
    let takes_focus = |index: i32| {
        list.row_at_index(index)
            .filter(|row| row.can_focus() && row.is_selectable())
    };
    // The same place, then onward, then back from it.
    let row = (at..)
        .map_while(|index| list.row_at_index(index).map(|_| index))
        .find_map(takes_focus)
        .or_else(|| (0..at).rev().find_map(takes_focus));
    match row {
        Some(row) => {
            list.select_row(Some(&row));
            row.grab_focus();
        }
        None => {
            if let Some(root) = list.root() {
                root.set_focus(None::<&gtk::Widget>);
            }
        }
    }
}
