//! Redrawing a list without losing the keyboard: a list that clears and rebuilds its rows destroys
//! the row holding the keyboard, leaving the window a focus widget with no parents, so `j`,
//! `Delete` or `Esc` do nothing until a click.

use gtk::prelude::*;

/// The keyboard goes to the row at the place of the one that had it, or the nearest that can take
/// it, or the window when the list is empty.
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

/// A list view draws a changed row as a new widget and destroys the old one, so the keyboard would
/// be left on nothing, or handed by GTK to whatever control came next (#2136, #2150). Hooked on the
/// model itself, before and after the view's own handler, so it holds for every change however many
/// loop turns the rows take to land.
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
