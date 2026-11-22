use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use postio_widgets::storyboard::deliver::{Delivery, press};

use super::storyboard_support::{display, show, until};

fn chord(text: &str) -> postio_ui::keymap::Chord {
    text.parse().expect("a chord")
}

/// An `adw::Window`, because a dialog is hosted by one.
fn window_with_list() -> (gtk::Window, gtk::ListBox, Rc<Cell<u32>>) {
    let window = adw::Window::new();
    let list = gtk::ListBox::new();
    for text in ["one", "two", "three"] {
        list.append(&gtk::Label::new(Some(text)));
    }
    window.set_content(Some(&list));
    window.set_default_size(300, 200);

    let seen = Rc::new(Cell::new(0));
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let counted = seen.clone();
    controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::j {
            counted.set(counted.get() + 1);
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(controller);
    (window.upcast(), list, seen)
}

pub fn a_key_reaches_the_window_from_inside_a_list() {
    if !display() {
        return;
    }
    let (window, list, seen) = window_with_list();
    show(&window);
    list.row_at_index(0).expect("a row").grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));

    let delivery = press(&window, &chord("j")).expect("a key");

    assert_eq!(seen.get(), 1, "the window's capture controller never saw j");
    assert!(
        matches!(delivery, Delivery::Delivered { .. }),
        "a claimed key was reported as {delivery:?}"
    );
}

pub fn a_dialog_over_the_window_keeps_the_key_from_it() {
    if !display() {
        return;
    }
    let (window, _list, seen) = window_with_list();
    show(&window);

    let dialog = adw::Dialog::new();
    let entry = gtk::Button::with_label("inside");
    dialog.set_child(Some(&entry));
    let dialog_saw = Rc::new(Cell::new(0));
    let controller = gtk::EventControllerKey::new();
    let counted = dialog_saw.clone();
    controller.connect_key_pressed(move |_, _, _, _| {
        counted.set(counted.get() + 1);
        gtk::glib::Propagation::Stop
    });
    dialog.add_controller(controller);
    dialog.present(Some(&window));
    assert!(until(|| entry.is_mapped()), "the dialog never opened");
    entry.grab_focus();
    assert!(until(|| adw::prelude::AdwDialogExt::focus(&dialog)
        .as_ref()
        == Some(entry.upcast_ref())));

    let delivery = press(&window, &chord("j")).expect("a key");

    assert_eq!(dialog_saw.get(), 1, "the dialog's controller never saw j");
    assert_eq!(seen.get(), 0, "the window saw a key the dialog consumed");
    assert!(
        matches!(&delivery, Delivery::Delivered { stopped_at } if stopped_at == "AdwDialog"),
        "expected the dialog to claim it, got {delivery:?}"
    );
}

pub fn a_key_with_the_keyboard_on_nothing_is_dropped_not_delivered() {
    if !display() {
        return;
    }
    let (window, list, seen) = window_with_list();
    show(&window);
    let row = list.row_at_index(1).expect("a row");
    row.grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));
    row.set_visible(false);
    list.remove(&row);
    let stranded = GtkWindowExt::focus(&window).expect("GTK kept the focus on the row");
    assert!(!stranded.is_mapped(), "the row is still on screen");

    let delivery = press(&window, &chord("j")).expect("a key");

    assert_eq!(delivery, Delivery::Dropped);
    assert_eq!(seen.get(), 0, "the window saw a key aimed at nothing");
}

pub fn return_in_a_text_field_activates_it() {
    if !display() {
        return;
    }
    let window = adw::Window::new();
    let entry = gtk::Entry::new();
    window.set_content(Some(&entry));
    window.set_default_size(300, 80);
    let activated = Rc::new(Cell::new(0));
    let counted = activated.clone();
    entry.connect_activate(move |_| counted.set(counted.get() + 1));
    let window: gtk::Window = window.upcast();
    show(&window);
    entry.grab_focus();
    assert!(until(|| GtkWindowExt::focus(&window).is_some()));

    let delivery = press(&window, &chord("Return")).expect("a key");

    assert!(
        matches!(&delivery, Delivery::Delivered { .. }),
        "Return in a focused field was {delivery:?}"
    );
    assert_eq!(activated.get(), 1, "the field was activated once");

    let _ = press(&window, &chord("j")).expect("a key");
    assert_eq!(activated.get(), 1, "only Return activates");
}
