//! `reachable` answers the class of defect delivery exists for (spec 008,
//! research R3): the keyboard is on a widget a person could not use. It is
//! observed on every step, whatever the delivery mode.

use adw::prelude::*;
use postio_gtk::storyboard::reach::reachable;

use super::storyboard_support::{display, show, until};

fn window_with_list() -> (adw::Window, gtk::ListBox) {
    let window = adw::Window::new();
    let list = gtk::ListBox::new();
    for text in ["one", "two"] {
        list.append(&gtk::Label::new(Some(text)));
    }
    window.set_content(Some(&list));
    window.set_default_size(300, 200);
    show(window.upcast_ref());
    (window, list)
}

pub fn a_focused_mapped_list_is_reachable() {
    if !display() {
        return;
    }
    let (window, list) = window_with_list();
    list.row_at_index(0).expect("a row").grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));

    assert!(reachable(window.upcast_ref()));
}

pub fn an_unmapped_focus_is_not_reachable() {
    if !display() {
        return;
    }
    let (window, list) = window_with_list();
    let row = list.row_at_index(0).expect("a row");
    row.grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));
    row.set_visible(false);
    assert!(until(|| !row.is_mapped()));

    assert!(!reachable(window.upcast_ref()));
}

pub fn a_modal_dialog_over_the_window_makes_it_unreachable() {
    if !display() {
        return;
    }
    let (window, list) = window_with_list();
    list.row_at_index(0).expect("a row").grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));
    assert!(
        reachable(window.upcast_ref()),
        "reachable before the dialog"
    );

    let dialog = adw::Dialog::new();
    let label = gtk::Label::new(Some("sure?"));
    dialog.set_child(Some(&label));
    dialog.present(Some(&window));
    assert!(until(|| label.is_mapped()), "the dialog never opened");

    assert!(
        !reachable(window.upcast_ref()),
        "a key aimed at the list would land on a dialog nobody asked it to"
    );
}

pub fn no_focus_widget_is_not_reachable() {
    if !display() {
        return;
    }
    let window = adw::Window::new();
    window.set_content(Some(&gtk::Label::new(Some("nothing focusable"))));
    window.set_default_size(300, 200);
    show(window.upcast_ref());
    // A freshly shown window may give focus to something; take it back.
    GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);

    assert!(!reachable(window.upcast_ref()));
}
