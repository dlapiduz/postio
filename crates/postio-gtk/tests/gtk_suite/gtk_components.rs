//! The shared components in `postio_gtk::widgets`, measured the way a person
//! would see them rather than by what they were handed.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::{fonts, style, widgets};

/// A presented window to put components in, or `None` with no display.
fn window() -> Option<gtk::Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = gtk::Window::new();
    style::track(&window);
    Some(window)
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn width(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.measure(gtk::Orientation::Horizontal, -1).1
}

/// A kicker reads in capitals whatever case its source was written in, so a
/// pane that passes "Theme" and one that passes "THEME" draw the same thing.
pub fn a_kicker_is_capitals_whatever_its_source_case() {
    let Some(window) = window() else {
        return;
    };
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let written = widgets::kicker("Check for mail");
    let shouted = widgets::kicker("CHECK FOR MAIL");
    column.append(&written);
    column.append(&shouted);
    window.set_child(Some(&column));
    window.present();
    pump();

    assert_eq!(
        width(&written),
        width(&shouted),
        "a sentence-case kicker should render as the capitals the canvas draws"
    );
    window.destroy();
}
