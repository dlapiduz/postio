//! The plain-text fallback (spec 006 FR-023; specs/007-postio-focus T218):
//! when it is drawn, and what it looks like.
//!
//! What is asserted is the snapshot the view holds -- what a person would
//! read -- never what the view was handed.

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use postio_render::Outcome;
use postio_widgets::body_view::BodyView;

use crate::support::{content, until};

/// A render that finished inside its deadline is the one shown, even when
/// the main loop came back to it late. The deadline and the poll for the
/// result are both main-loop timers; a main thread busy past the deadline
/// (sanitising the next message, laying out the dialog, a sync landing)
/// found both due at once, ran the deadline first, and replaced a finished
/// render with the plain-text fallback.
pub fn a_finished_render_is_shown_when_the_main_loop_was_late() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let deadline = Duration::from_millis(200);
    let view = BodyView::new(deadline);
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    assert!(until(|| view.width() > 0), "the view was never allocated");

    view.set_content_from_top(content("plain-text-simple"));
    // The render runs on its own thread and is done long before this ends;
    // the main loop is what is late.
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(2)));
    assert!(
        until(|| view.document().is_some()),
        "nothing was ever shown"
    );
    let outcome = view.document().expect("a snapshot").outcome;
    window.close();
    assert_eq!(
        outcome,
        Outcome::Rendered,
        "a render that had finished was replaced by the fallback"
    );
}
