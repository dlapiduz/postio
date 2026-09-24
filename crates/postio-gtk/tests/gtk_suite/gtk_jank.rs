//! A handler that holds the main loop past the frame budget is reported,
//! with the action that ran just before it.
//!
//! The unit tests in `jank.rs` prove the judgement; this proves the probe is
//! actually watching -- that a heartbeat on a real main loop notices a
//! handler blocking it, which is the stall a frame clock never sees.
//!
//! Skips without a display. Nothing here touches the network.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::jank;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("not poisoned").clone()).expect("utf-8")
    }
}

pub fn a_blocked_main_loop_is_reported_with_the_action_before_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
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
    // The shape of the bug this exists to find: a key handler that reads the
    // store on the main thread.
    glib::idle_add_local_once(|| std::thread::sleep(Duration::from_millis(40)));

    let context = glib::MainContext::default();
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_millis(3000));
    while !captured.text().contains("archive") && Instant::now() < deadline {
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
