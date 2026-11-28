use std::time::{Duration, Instant};

use gtk::prelude::*;

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

pub fn show(window: &gtk::Window) {
    window.present();
    assert!(until(|| window.is_mapped()), "the window never mapped");
}

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
