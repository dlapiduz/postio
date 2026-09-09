//! The conversation rail (#1374): the column that says where you are in a
//! thread.
//!
//! What needs a display is what a person would see — that there is a row per
//! message, that the row carries its number and sender, that a length appears
//! only where one is worth saying, and that the marked row is drawn as marked
//! rather than merely recorded as marked. The rule that decides *which* row is
//! marked is arithmetic and lives in `postio_ui::reader::rail`, where #1359
//! and #1372 prove it without a display.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::rail::RailColumn;
use postio_gtk::{fonts, style};
use postio_ui::reader::rail::{LENGTH_THRESHOLD, rows};

/// A realized rail, or `None` with no display.
fn rail() -> Option<(gtk::Window, RailColumn)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let rail = RailColumn::new();
    let window = gtk::Window::new();
    window.set_child(Some(rail.widget()));
    window.present();
    Some((window, rail))
}

/// Every label in the tree, in order, so a case can assert on what is on
/// screen rather than on what the widget was handed.
fn labels(root: &gtk::Widget) -> Vec<String> {
    of_class(root, None)
}

/// The labels carrying `class`, in order.
///
/// By class rather than by text, because a row number and a line count are
/// both small integers: the first version of this file asserted that no label
/// read `"4"` to prove a four-line message shows no length, and message *four*
/// made it fail. Position and length are different things that happen to look
/// alike, and only the class says which is which.
fn of_class(root: &gtk::Widget, class: Option<&str>) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = root.first_child();
    while let Some(child) = next {
        if let Some(label) = child.downcast_ref::<gtk::Label>() {
            if class.is_none_or(|class| label.has_css_class(class)) {
                found.push(label.label().to_string());
            }
        }
        found.extend(of_class(&child, class));
        next = child.next_sibling();
    }
    found
}

pub fn the_rail_lists_a_thread_and_marks_what_is_on_screen() {
    let Some((window, rail)) = rail() else {
        return;
    };

    let senders: Vec<String> = ["Tessa Vaughn", "me", "Tessa Vaughn", "me"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    // The third message is the essay; the rest are short, and the last has no
    // body yet -- three different reasons for a row to carry no number.
    let lengths = [Some(4), Some(LENGTH_THRESHOLD - 1), Some(84), None];
    rail.set_thread(&rows(&senders, &lengths));
    rail.set_marked(Some(2));

    let seen = labels(rail.widget());
    let joined = seen.join(" | ");

    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-number")),
        vec!["1", "2", "3", "4"],
        "a row per message, numbered as a person counts: {joined}"
    );
    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-sender")),
        senders,
        "every row says who wrote it: {joined}"
    );
    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-length")),
        vec!["84"],
        "only the essay is long enough to be worth a number -- the four-line \
         message, the one just under the threshold, and the one whose body \
         has not arrived all show nothing: {joined}"
    );
    assert!(
        seen.iter().any(|text| text.contains("In this thread")),
        "the rail names itself: {joined}"
    );
    assert!(
        seen.iter().any(|text| text.contains("3 of 4")),
        "the footer states the position: {joined}"
    );

    assert_eq!(
        rail.marked_position(),
        Some(3),
        "the marked row is the one the rule chose, counted as a person counts"
    );
    assert!(
        rail.row_is_marked(2),
        "the marked row must be drawn as marked, not merely recorded"
    );
    assert!(
        !rail.row_is_marked(0),
        "exactly one row is marked at a time"
    );

    window.close();
}

pub fn activating_a_row_reports_the_message_it_names() {
    let Some((window, rail)) = rail() else {
        return;
    };

    rail.set_thread(&rows(
        &["Ada".to_owned(), "Grace".to_owned(), "Katherine".to_owned()],
        &[None, None, None],
    ));

    let seen = Rc::new(RefCell::new(Vec::new()));
    rail.connect_activated({
        let seen = Rc::clone(&seen);
        move |index| seen.borrow_mut().push(index)
    });

    rail.activate_row(1);

    assert_eq!(
        seen.borrow().as_slice(),
        &[1],
        "activating a row reports the message it names, once"
    );

    window.close();
}
