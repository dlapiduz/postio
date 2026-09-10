//! Where the keyboard is when the window opens (#1473).
//!
//! Reported from real use: the window comes up with the caret in the search
//! field, so `j`, `a` and `?` type characters into it instead of acting on
//! the mail. The first thing every launch needs is a click.
//!
//! `Window::present` already means to prevent exactly this — it grabs the
//! list unless something has claimed focus already (#614) — and
//! `gtk_finder_focus.rs` shows `?` working on a bare window, so on the widget
//! layer alone it looks fine. What that misses is the order the real
//! application does things in: `app.rs` presents the window *before* any mail
//! exists, and a list with no rows has nothing to focus. Whatever the grab
//! did or did not do, nothing revisits the question once the mail lands.
//!
//! So this drives the real sequence — present an empty window, feed it, let
//! it settle — and asks the window what has focus. Asserting on the focused
//! widget rather than on `grab_focus` having been called is the point: the
//! call was already there while the bug was being reported.
//!
//! # It passed the first time it ran, and that is why there is a control
//!
//! The hypothesis in #1473 was that the pre-`present` grab does not stick on
//! an unrealized, empty list. It does: this reports `GtkListView` even when
//! it mirrors `app.rs` exactly. So this test has never been red, and
//! `CLAUDE.md` is clear that a test which has not been seen to fail is one
//! nobody should trust.
//!
//! Rather than break the window to find out, the control below puts the
//! keyboard in the search field — the state being reported — and asserts the
//! predicate is false there. That proves the assertion distinguishes the two
//! states, which is the property a green run needs to mean anything, and it
//! costs one grab.
//!
//! One test function, for the reason the other app_suite cases give: GTK
//! initialises once per process.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, actions};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// The header's search entry, found by walking the tree — the same way
/// `gtk_suite/gtk_finder_focus.rs` reaches it, and for the same reason: the
/// field is a `GtkText` inside a styled box rather than anything the window
/// hands out.
fn search_field(window: &Window) -> gtk::Text {
    fn find(widget: &gtk::Widget) -> Option<gtk::Text> {
        if let Some(text) = widget.downcast_ref::<gtk::Text>() {
            return Some(text.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.upcast_ref::<gtk::Widget>()).expect("the header has a search field")
}

/// What has the keyboard, named well enough to read in a failure.
fn focused(window: &Window) -> String {
    match gtk::prelude::GtkWindowExt::focus(window) {
        None => "nothing".to_owned(),
        Some(widget) => {
            let kind = widget.type_().name().to_owned();
            let classes: Vec<String> = widget
                .css_classes()
                .iter()
                .map(|class| class.to_string())
                .collect();
            if classes.is_empty() {
                kind
            } else {
                format!("{kind} [{}]", classes.join(" "))
            }
        }
    }
}

pub fn the_window_opens_with_the_keyboard_on_the_first_message() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    seed_small(&database, 11);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    // The order `app.rs` uses: the window is presented before there is any
    // mail to put in it.
    let window = Window::default();
    // The two things `app.rs` does between building the window and
    // presenting it. Both add focusable widgets to the tree, and the bug
    // report is about which one GTK settles on.
    postio_gtk::config::install(&window);
    window.composer();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let _feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "no rows arrived, so this test cannot say where the keyboard should be"
    );

    // The control. Put the keyboard where the report says it lands, and
    // check the predicate below can tell: without this, a green run only
    // says the assertion is satisfiable, not that it is selective.
    let on_the_list = |window: &Window| {
        let list = window.list();
        list.has_focus() || list.focus_child().is_some()
    };
    search_field(&window).grab_focus();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        !on_the_list(&window),
        "with the keyboard in the search field this test still reports the \
         list, so it cannot tell the reported state from the wanted one and \
         a green run means nothing (focus: {})",
        focused(&window)
    );

    // Back to how the window came up, and then the real question.
    window.list().grab_focus();
    while glib::MainContext::default().iteration(false) {}

    // Reported before the assertion: when this fails, the *name* of what
    // stole the keyboard is the whole finding.
    eprintln!("focus after launch: {}", focused(&window));

    assert!(
        list.has_focus() || list.focus_child().is_some(),
        "the keyboard is on {} rather than the message list, so every \
         single-key binding types into it instead of acting on the mail \
         (#1473)",
        focused(&window)
    );
}
