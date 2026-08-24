//! Opening a message fills the reading pane, and its chips open the tree.
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
    settle_for(std::time::Duration::from_secs(10), done)
}

/// As [`settle_until`], with a deadline of your own.
///
/// The generous default is right when the next thing to happen is the thing
/// being waited for. It is wrong when the wait is a *probe* — asking whether
/// this row has attachments — because every miss then costs the full budget.
fn settle_for(budget: std::time::Duration, done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + budget;
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
fn opening_a_message_fills_the_pane_and_its_chips_open_the_parts_tree() {
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

    // ── the chips, and the tree behind them ─────────────────────────────
    //
    // `postio-v62`. The MIME tree was built, tested and rendered, and nothing
    // in the running application could open it: the only entry point is a
    // chip in a reader, and until `postio-y39y` there was no reader.
    let with_parts = settle_until_row(&window, |window| !chips(window).is_empty());
    assert!(
        with_parts,
        "no message in the corpus showed an attachment chip; either the seed          has no attachments or the chips are not being fed"
    );

    assert!(
        !window.parts().is_visible(),
        "the panel is what a chip opens, not something already open"
    );
    chips(&window)[0].emit_clicked();
    while glib::MainContext::default().iteration(false) {}

    assert!(
        window.parts().is_visible(),
        "a chip asks; the panel is where the verbs live"
    );
    let nodes = window.parts().nodes();
    assert!(
        nodes.len() > 1,
        "the tree should hold a root and at least one part, not {}",
        nodes.len()
    );
    assert!(
        nodes.iter().any(|node| node.is_leaf()),
        "a tree of nothing but containers means the parts never arrived"
    );

    // ── and none of it fetched anything ─────────────────────────────────
    //
    // The seed marks every message `BodyState::NotFetched` and this test
    // never starts an engine, so there is nothing on this machine to draw
    // from. The panel opening anyway is the point: it is drawn from
    // `BODYSTRUCTURE` metadata, which is what lets a message that has never
    // been downloaded still say what came with it.
    assert!(
        nodes.iter().all(|node| !node.downloaded),
        "nothing was downloaded, so no node should claim to be"
    );

    bridge.shutdown();
}

/// Every attachment chip currently under the message body.
fn chips(window: &Window) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.has_css_class("postio-attachment")
        {
            found.push(button.clone());
        }
    });
    found
}

/// Activate rows in turn until `done`, because only some messages in the
/// corpus carry attachments and which one lands where depends on the seed.
fn settle_until_row(window: &Window, done: impl Fn(&Window) -> bool) -> bool {
    let list = window.list();
    for position in 0..list.model().n_items().min(40) {
        activate_row(window, position);
        // A probe, not a wait: the read is local and the chips are drawn on
        // the same reply as the body, so a row that has not answered in this
        // long has nothing to show.
        if settle_for(std::time::Duration::from_millis(400), || done(window)) {
            return true;
        }
    }
    false
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
}

/// Activate the list's first row by emitting the signal GTK itself emits for
/// a double click or `Enter`.
///
/// Reached through the widget tree rather than through an accessor: the inner
/// `gtk::ListView` is `postio-gtk`'s private business, and a test that made it
/// public would be widening the API to observe it.
fn activate_first_row(window: &Window) {
    activate_row(window, 0);
}

fn activate_row(window: &Window, position: u32) {
    let view = find_list_view(window.upcast_ref::<gtk::Widget>())
        .expect("the message list is built on a GtkListView");
    view.emit_by_name::<()>("activate", &[&position]);
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
