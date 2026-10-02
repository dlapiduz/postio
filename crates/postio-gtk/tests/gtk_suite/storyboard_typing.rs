//! Typed text goes where the keyboard is (spec 008, research R3), and a
//! keyboard on nothing that takes text is a failure, not a silent no-op.

use gtk::prelude::*;
use postio_gtk::storyboard::deliver::{TypeOutcome, type_text};

use super::storyboard_support::{display, show, until};

fn shown(child: &impl IsA<gtk::Widget>) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_child(Some(child));
    window.set_default_size(300, 200);
    show(&window);
    window
}

fn focus(window: &gtk::Window, widget: &impl IsA<gtk::Widget>) {
    widget.grab_focus();
    assert!(until(|| GtkWindowExt::focus(window).is_some()));
}

pub fn text_goes_in_at_the_cursor_of_a_focused_entry() {
    if !display() {
        return;
    }
    let entry = gtk::Text::new();
    entry.set_text("hello world");
    let window = shown(&entry);
    focus(&window, &entry);
    entry.set_position(5);

    let outcome = type_text(&window, ",", None);

    assert_eq!(outcome, TypeOutcome::Typed);
    assert_eq!(entry.text(), "hello, world");
    assert_eq!(entry.position(), 6, "the cursor stayed behind the text");
}

pub fn text_goes_in_at_the_insert_mark_of_a_focused_text_view() {
    if !display() {
        return;
    }
    let view = gtk::TextView::new();
    view.buffer().set_text("ab");
    let window = shown(&view);
    focus(&window, &view);
    view.buffer().place_cursor(&view.buffer().iter_at_offset(1));

    let outcome = type_text(&window, "X", None);

    assert_eq!(outcome, TypeOutcome::Typed);
    let buffer = view.buffer();
    let all = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    assert_eq!(all, "aXb");
}

pub fn a_hook_takes_what_no_editable_does() {
    if !display() {
        return;
    }
    let button = gtk::Button::with_label("a web body, say");
    let window = shown(&button);
    focus(&window, &button);
    let taken = std::cell::RefCell::new(String::new());
    let hook = |_: &gtk::Widget, text: &str| {
        taken.borrow_mut().push_str(text);
        true
    };

    let outcome = type_text(&window, "hi", Some(&hook));

    assert_eq!(outcome, TypeOutcome::Typed);
    assert_eq!(*taken.borrow(), "hi");
}

pub fn a_keyboard_on_a_list_has_nothing_to_type_into() {
    if !display() {
        return;
    }
    let list = gtk::ListBox::new();
    list.append(&gtk::Label::new(Some("row")));
    let window = shown(&list);
    focus(&window, &list.row_at_index(0).expect("a row"));

    assert_eq!(
        type_text(&window, "hi", None),
        TypeOutcome::NothingToTypeInto
    );
}
