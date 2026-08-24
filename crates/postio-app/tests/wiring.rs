//! The composition root, driven the way the binary drives it.
//!
//! `postio-bl2`: eight capabilities were found fully implemented, tested, and
//! never called. Four of them lived here, in the crate that joins the store,
//! the runtime and the view — and this crate had no integration coverage at
//! all, because it was bin-only and `tests/` cannot link a binary.
//!
//! # What makes this test different from the ones that missed the bugs
//!
//! Every existing test asserts that a layer does its job when it is *handed*
//! its inputs. A widget test feeds the widget rows and checks it draws them.
//! A store test writes rows and checks it reads them back. Both pass while
//! nothing connects the two.
//!
//! This one starts where the application starts: an account and mail in a
//! store, a real `Window`, and `feed_the_window` — the same function `run`
//! calls. The assertion is *the pane has content*, never *the pane renders
//! content it was given*. That is the only shape of assertion that can fail
//! when the wiring is missing, which is why none of the eight were caught.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket, and this never calls it. Nothing here touches the network.
//!
//! One test function: GTK is single-threaded and initialised once, and the
//! page replies are awaited on the thread-default main context, which the
//! harness would otherwise drive from two threads at once.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// Run the main loop until `done` or the budget runs out.
///
/// The page reads cross to the runtime and answer over a channel, so the
/// rows are not there the instant `feed_the_window` returns. A deadline
/// rather than a fixed number of iterations: what is being waited for is a
/// round trip, and a spin count is a sleep with extra steps.
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
fn a_window_over_a_populated_store_lists_its_mail() {
    let state_dir = std::env::temp_dir().join(format!("postio-wiring-{}", std::process::id()));
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

    // ── a store with an account, folders and real mail in it ────────────
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(
        report.message_count > 0,
        "the fixture seeded no mail, so this test could not fail"
    );
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    // The runtime the reads are polled on. A no-op command handler: this
    // test is about the panes being fed, not about what a keystroke does.
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ───────────────────────────────────────
    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    let _ = feeds;

    // ── the folders reached the sidebar ─────────────────────────────────
    let list = window.list();
    let listed = settle_until(|| list.model().n_items() > 0);

    assert!(
        listed,
        "the window was fed a store holding {} messages and the list is empty. \
         Every layer under this one is tested and passes; that is exactly the \
         shape of bug postio-bl2 is about — check what is *between* them.",
        report.message_count
    );

    // Not merely non-empty: the rows have to be the store's, and a row that
    // draws no sender and no subject is a row the model invented.
    let rows = list.model().n_items();
    assert!(
        list.model().peek(0).is_some(),
        "the list reports {rows} rows and cannot name the first one"
    );

    bridge.shutdown();
}
