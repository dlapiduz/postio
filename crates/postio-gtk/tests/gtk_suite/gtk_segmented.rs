//! The segmented control (#1179): a closed set with every answer on screen.
//!
//! The three behaviours a pane depends on — that exactly one segment is
//! active, that redrawing from the file does not write back to it, and that
//! one press reports one change — none of which a `gtk::DropDown` needed
//! because a dropdown was never a group. Skips without a display. Nothing
//! here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::widgets::SegmentedControl;
use postio_gtk::{fonts, style};

/// A realized control, or `None` with no display.
fn control() -> Option<(gtk::Window, Rc<SegmentedControl>)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let control = Rc::new(SegmentedControl::new("Theme", &["System", "Light", "Dark"]));
    let window = gtk::Window::new();
    window.set_child(Some(control.widget()));
    window.present();
    Some((window, control))
}

pub fn setting_the_value_moves_the_group_without_reporting_a_change() {
    let Some((window, control)) = control() else {
        return;
    };

    let seen = Rc::new(RefCell::new(Vec::new()));
    control.connect_selected({
        let seen = Rc::clone(&seen);
        move |index| seen.borrow_mut().push(index)
    });

    control.set_selected(2);

    assert_eq!(control.selected(), Some(2), "the file's value is showing");
    assert!(
        seen.borrow().is_empty(),
        "redrawing from the file must not write back to it, but reported {:?}",
        seen.borrow()
    );

    window.destroy();
}

pub fn pressing_a_segment_reports_it_exactly_once() {
    let Some((window, control)) = control() else {
        return;
    };

    control.set_selected(0);
    let seen = Rc::new(RefCell::new(Vec::new()));
    control.connect_selected({
        let seen = Rc::clone(&seen);
        move |index| seen.borrow_mut().push(index)
    });

    control.test_press(1);

    assert_eq!(
        *seen.borrow(),
        vec![1],
        "one press is one change -- the segment that switched off is not a \
         second one"
    );
    assert_eq!(control.selected(), Some(1));

    window.destroy();
}

pub fn pressing_the_active_segment_changes_nothing() {
    let Some((window, control)) = control() else {
        return;
    };

    control.set_selected(1);
    let seen = Rc::new(RefCell::new(Vec::new()));
    control.connect_selected({
        let seen = Rc::clone(&seen);
        move |index| seen.borrow_mut().push(index)
    });

    control.test_press(1);

    assert!(
        seen.borrow().is_empty(),
        "pressing what is already chosen is not a choice, but reported {:?}",
        seen.borrow()
    );
    assert_eq!(control.selected(), Some(1), "and it stays chosen");

    window.destroy();
}
