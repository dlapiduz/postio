//! The status line's sync button reaches the same verb `F5` does (#495).
//!
//! Reported directly: *"when downloading stuff the indicator hides the sync
//! button. I want to be able to sync even with longer processes running in
//! the background."* `Refresh` was always a real command, correctly wired;
//! what it had was no persistent surface. The only hint on screen lived in
//! `list_state`'s banner, drawn for `Offline` and `Failing` and hidden the
//! moment the account connects — so the affordance disappeared exactly when
//! a long backfill made somebody want it.
//!
//! `gtk_sidebar.rs` proves the control is there and stays there through
//! every connection state, without an application around it. What it cannot
//! prove is that pressing it *does* anything: the sidebar only reports the
//! ask, and something has to turn that into a command on the bus. That is
//! this file, and it is the `postio-bl2` shape — a control that reports into
//! nothing looks identical to one that works.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_core::{Command, CommandId};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn the_status_lines_sync_button_asks_for_a_refresh() {
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
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // Every command that reaches the bus, recorded. `Refresh` starts a
    // network pass in the real application, which this test has no business
    // doing — what it asserts is that the ask *arrives*. Behind a lock
    // because the bus runs handlers on its own runtime, not this thread.
    let asked: Arc<Mutex<Vec<CommandId>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&asked);
    let (bridge, _replies) = Bridge::new(handler_fn(move |command: Command, _| {
        recorder.lock().expect("not poisoned").push(command.id());
        async {}
    }))
    .expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    // `Refresh` has to be wired, or `commands::install`'s own filter drops it
    // before it can reach anything — which would make this test pass for the
    // wrong reason if it asserted on the button alone.
    commands::install(
        &window,
        &feeds,
        Default::default(),
        wiring.commands.clone(),
        vec![CommandId::Refresh],
    );

    // ── the pointer ──────────────────────────────────────────────────────
    let button = refresh_button(&window).expect("the status line offers a manual sync");
    assert!(
        button.is_visible() && button.is_sensitive(),
        "the trigger is on screen but cannot be pressed"
    );
    button.emit_clicked();

    assert!(
        settle_until(|| asked
            .lock()
            .expect("not poisoned")
            .contains(&CommandId::Refresh)),
        "the sync button reported into nothing: it is drawn, it is clickable, \
         and no command reaches the bus — which is exactly what a control \
         wired to nobody looks like ({:?})",
        asked.lock().expect("not poisoned")
    );

    // ── and the key, to the same place ───────────────────────────────────
    // The acceptance asks for identical reach, so this asserts they are the
    // same verb rather than two paths that happen to both work.
    asked.lock().expect("not poisoned").clear();
    window.handle_key(gdk::Key::F5, gdk::ModifierType::empty());
    assert!(
        settle_until(|| asked
            .lock()
            .expect("not poisoned")
            .contains(&CommandId::Refresh)),
        "`F5` no longer reaches the same command the button does: {:?}",
        asked.lock().expect("not poisoned")
    );

    bridge.shutdown();
}

fn refresh_button(window: &Window) -> Option<gtk::Button> {
    fn walk(widget: &gtk::Widget, found: &mut Option<gtk::Button>) {
        if found.is_none()
            && widget.has_css_class("postio-status-refresh")
            && let Ok(button) = widget.clone().downcast::<gtk::Button>()
        {
            *found = Some(button);
        }
        let mut child = widget.first_child();
        while let Some(node) = child {
            walk(&node, found);
            child = node.next_sibling();
        }
    }
    let mut found = None;
    walk(window.upcast_ref::<gtk::Widget>(), &mut found);
    found
}
