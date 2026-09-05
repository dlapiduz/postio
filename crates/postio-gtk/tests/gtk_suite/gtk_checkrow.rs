//! The checkbox that replaced six switches (#1179).
//!
//! One behaviour, and it is the one every settings pane depends on: showing
//! the file's current value must not count as changing it. Skips without a
//! display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::widgets::CheckRow;
use postio_gtk::{fonts, style};

/// A realized checkbox, or `None` with no display.
fn check_row() -> Option<(gtk::Window, Rc<CheckRow>)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let row = Rc::new(CheckRow::new("Block remote images and trackers"));
    let window = gtk::Window::new();
    window.set_child(Some(row.widget()));
    window.present();
    Some((window, row))
}

pub fn showing_the_files_value_is_not_changing_it() {
    let Some((window, row)) = check_row() else {
        return;
    };

    let seen = Rc::new(RefCell::new(Vec::new()));
    row.connect_toggled({
        let seen = Rc::clone(&seen);
        move |active| seen.borrow_mut().push(active)
    });

    row.set_active(true);

    assert!(row.is_active(), "the file's value is showing");
    assert!(
        seen.borrow().is_empty(),
        "redrawing from the file must not write back to it, but reported {:?}",
        seen.borrow()
    );

    window.destroy();
}

pub fn a_person_toggling_it_is_reported_once() {
    let Some((window, row)) = check_row() else {
        return;
    };

    row.set_active(false);
    let seen = Rc::new(RefCell::new(Vec::new()));
    row.connect_toggled({
        let seen = Rc::clone(&seen);
        move |active| seen.borrow_mut().push(active)
    });

    row.widget().emit_activate();

    assert_eq!(
        *seen.borrow(),
        vec![true],
        "one press, one change, reported"
    );

    window.destroy();
}
