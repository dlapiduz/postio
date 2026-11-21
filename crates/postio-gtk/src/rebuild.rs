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

/// Build a list view over `model` with `build`, and keep the keyboard in it
/// whenever `model`'s rows change. A list view draws a changed row as a new
/// widget and destroys the old one, and the keyboard on it would be left on
/// nothing -- or handed by GTK to whatever control came next -- until
/// something put it back (#2136, #2150). It goes back to the row now at the
/// place of the one that had it: the view's own idea of where its keyboard
/// is, which it keeps across the change.
///
/// Hooked on the model itself, before and after the view's own handler, so
/// it holds for every change, whoever makes it and however many turns of
/// the loop the rows take to land.
pub(crate) fn keeping_keyboard(
    model: &impl IsA<gtk::gio::ListModel>,
    build: impl FnOnce() -> gtk::ListView,
) -> gtk::ListView {
    fn within(view: &gtk::ListView) -> bool {
        view.root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| {
                focus == *view.upcast_ref::<gtk::Widget>() || focus.is_ancestor(view)
            })
    }
    let watched: std::rc::Rc<gtk::glib::WeakRef<gtk::ListView>> = std::rc::Rc::default();
    let had = std::rc::Rc::new(std::cell::Cell::new(false));
    // Connected before the view connects its own: this runs first, while
    // the row with the keyboard is still there.
    model.connect_items_changed({
        let watched = std::rc::Rc::clone(&watched);
        let had = std::rc::Rc::clone(&had);
        move |_, _, _, _| had.set(watched.upgrade().is_some_and(|view| within(&view)))
    });
    let view = build();
    watched.set(Some(&view));
    model.connect_items_changed({
        let watched = std::rc::Rc::clone(&watched);
        move |_, _, _, _| {
            if had.take()
                && let Some(view) = watched.upgrade()
                && !within(&view)
            {
                view.grab_focus();
            }
        }
    });
    view
}
