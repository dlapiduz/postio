//! `Window::run_search`: activating a saved search from outside the box.
//!
//! Issue #10's sidebar entry does not type into the field -- it hands the
//! window a query string directly, the same shape of "outside a click" seam
//! `Window::open_mailbox`/`open_message` are for a notification (see
//! `gtk_window_open_message.rs`). This is what it has to do: open the box in
//! search mode, show the query as though it had been typed, and run it *now*
//! rather than waiting out the debounce a keystroke would -- a saved search
//! that took 300ms to answer would look broken next to every other row.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.iteration(false) {}
}

#[test]
fn run_search_opens_the_box_and_answers_immediately() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    let finder = window.finder();
    let live = finder
        .live()
        .expect("the box has a field, so it has a readout");

    let asked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    live.connect_run({
        let asked = asked.clone();
        move |query, _sequence| asked.borrow_mut().push(query.input().to_owned())
    });

    assert!(!finder.is_open(), "closed before anything asks it to open");

    window.run_search("is:unread from:team");
    pump();

    assert!(finder.is_open(), "activating a saved search opens the box");
    assert_eq!(finder.mode(), Mode::Search);
    assert_eq!(
        finder.query().text,
        "is:unread from:team",
        "the box should show the saved query, not an empty field"
    );
    assert_eq!(
        asked.borrow().as_slice(),
        ["is:unread from:team"],
        "the query must run right away -- a saved search cannot be made to \
         wait out the debounce a keystroke would"
    );

    // Running a different saved search replaces the box's contents rather
    // than piling a second query behind the first.
    window.run_search("has:attach");
    pump();
    assert_eq!(finder.query().text, "has:attach");
    assert_eq!(
        asked.borrow().as_slice(),
        ["is:unread from:team", "has:attach"]
    );

    window.destroy();
}
