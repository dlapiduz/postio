//! Opening a message fills the reading pane.
//!
//! `postio-y39y`. Postio could list mail and not read it: nothing mounted a
//! `Reader` into the pane the PLATE layout gives it, so selecting a message
//! left the right-hand column blank. Every layer under this one passed —
//! the reader renders bodies, the store holds them, the list draws rows — and
//! nothing joined them, which is the shape of bug `postio-bl2` exists for.
//!
//! So the assertion here is *the pane has a message in it*, never *the reader
//! renders a body it was handed*. Only the first kind can fail when the
//! wiring is missing.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket, and this never calls it. A body that has not been fetched
//! simply does not draw.
//!
//! One test function, for the reason `wiring.rs` gives.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

#[test]
fn opening_a_message_puts_it_in_the_reading_pane() {
    let state_dir = std::env::temp_dir().join(format!("postio-reading-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ───────────────────────────────────────
    let wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let _ = &wired;

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the list is empty, so there is nothing to open"
    );

    // ── nothing is being read yet ───────────────────────────────────────
    assert!(
        !window.reading(),
        "an untouched window shows the pane's empty state, not a message"
    );

    // ── open the first row, the way a double click or `Enter` does ──────
    activate_first_row(&window);

    assert!(
        settle_until(|| window.reading()),
        "the message was activated and the reading pane never filled. The \
         reader renders bodies and the store holds them; what is missing is \
         between them."
    );
    assert!(
        window.reader().widget().is_visible(),
        "the pane says it is reading and the reader is not on screen"
    );

    bridge.shutdown();
}

/// Activate the list's first row by emitting the signal GTK itself emits for
/// a double click or `Enter`.
///
/// Reached through the widget tree rather than through an accessor: the inner
/// `gtk::ListView` is `postio-gtk`'s private business, and a test that made it
/// public would be widening the API to observe it.
fn activate_first_row(window: &Window) {
    let view = find_list_view(window.upcast_ref::<gtk::Widget>())
        .expect("the message list is built on a GtkListView");
    view.emit_by_name::<()>("activate", &[&0u32]);
}

fn find_list_view(widget: &gtk::Widget) -> Option<gtk::ListView> {
    if let Some(view) = widget.downcast_ref::<gtk::ListView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(found) = find_list_view(&node) {
            return Some(found);
        }
        child = node.next_sibling();
    }
    None
}
