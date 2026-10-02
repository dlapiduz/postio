//! Whether the keyboard is on something a person could use.
//!
//! Observed on every step whatever the delivery mode (research R3): it is a
//! cheap guard on the class of defect `chain` exists for, and it makes "the
//! keyboard is on nothing" a check rather than a guess.

use adw::prelude::AdwDialogExt;
use gtk::prelude::*;

use super::deliver::presented_dialog;

/// Whether a key pressed now would reach a widget a person can see: the
/// keyboard's widget exists, is shown, belongs to this window, and nothing
/// modal is presented over it.
///
/// While a dialog is up the keyboard is the dialog's (it keeps its own
/// focus), so the dialog counts only when something inside it holds focus:
/// a dialog with nothing focused is a covered window, and a key lands on
/// nothing. A separate modal window over this one is covering it outright.
pub fn reachable(window: &gtk::Window) -> bool {
    let modal_over = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|toplevel| toplevel.downcast::<gtk::Window>().ok())
        .any(|other| {
            other != *window
                && other.is_modal()
                && other.is_visible()
                && other.transient_for().as_ref() == Some(window)
        });
    if modal_over {
        return false;
    }
    let focus = match presented_dialog(window) {
        Some(dialog) => AdwDialogExt::focus(&dialog),
        None => GtkWindowExt::focus(window),
    };
    focus.is_some_and(|widget| {
        widget.is_mapped()
            && widget.root().is_some_and(|root| {
                root.upcast_ref::<gtk::Widget>() == window.upcast_ref::<gtk::Widget>()
            })
    })
}
