use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use postio_widgets::jank;

use postio_test_support::logs::Captured;

pub fn a_blocked_main_loop_is_reported_with_the_action_before_it() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    let _default = tracing::subscriber::set_default(subscriber);

    let window = gtk::Window::new();
    window.set_default_size(200, 100);
    jank::install(&window);
    window.present();

    jank::note_action("archive");
    glib::idle_add_local_once(|| std::thread::sleep(Duration::from_millis(40)));

    let context = glib::MainContext::default();
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_millis(3000));
    // Wait for the main loop's report: under load a slow frame is reported with the same action
    // first.
    while !captured.text().contains("main loop") && Instant::now() < deadline {
        context.iteration(true);
    }
    let log = captured.text();
    window.close();

    assert!(
        log.contains("over the frame budget") && log.contains("main loop"),
        "a 40 ms handler went unreported:\n{log}"
    );
    assert!(
        log.contains("action=\"archive\"") || log.contains("action=archive"),
        "the stall was reported without the action before it:\n{log}"
    );
}
