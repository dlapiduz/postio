//! Helpers the `storyboard_*` cases share. Holds no case of its own.

use std::time::{Duration, Instant};

use gtk::prelude::*;

/// Whether there is a display to draw on; the case skips itself without one.
pub fn display() -> bool {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    true
}

/// Turn the main loop until `condition` holds, or the (scaled) deadline.
pub fn until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(Duration::from_millis(10), || {
        gtk::glib::ControlFlow::Continue
    });
    let held = loop {
        if condition() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        context.iteration(true);
    };
    heartbeat.remove();
    held
}

/// Present `window` and wait until it is mapped.
pub fn show(window: &gtk::Window) {
    window.present();
    assert!(until(|| window.is_mapped()), "the window never mapped");
}

/// Turn the main loop for `span`, whatever happens.
pub fn run_for(span: Duration) {
    let end = Instant::now() + postio_test_support::scaled(span);
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(Duration::from_millis(5), || {
        gtk::glib::ControlFlow::Continue
    });
    while Instant::now() < end {
        context.iteration(true);
    }
    heartbeat.remove();
}
