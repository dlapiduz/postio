//! `Return` and `Tab` from the search box move the keyboard to the message
//! list, on a real display -- the wiring lives in `postio-app` (which owns
//! `Window::list`), not in `postio-gtk` (#693).
//!
//! `Return` calls `Finder::activate`, which flushes the debounced query and
//! fires `on_search` -- unwired before this issue, so the keyboard stayed in
//! the field no matter how a search ran. `Tab` already moved to the first
//! refine chip when there was one (`postio_gtk::search::View::attach`); this
//! is the fallback for when there is not, replacing whatever the raw GTK
//! focus chain used to land on.
//!
//! One test function: GTK is single-threaded and initialised once. See
//! `search_wiring.rs`, whose fixture and settle pattern this reuses.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A word every fixture in the corpus carries in a header, so a query
/// reliably answers with at least one hit -- there is no refine chip to
/// assert on here, so unlike `search_wiring.rs` this test does not need the
/// facets pass to have finished, only the readout.
const QUERY: &str = "example.com";

/// Whether the message list holds the keyboard.
///
/// `is_focus`, not `has_focus`, for the same reason `Finder::has_keyboard`
/// gives: the question is which widget the keyboard is on in this window, not
/// whether the window is the active one. `focus_child` covers the actual
/// destination -- `grab_focus` on the list delegates to whichever row or
/// internal child GTK's own focus handling picks, so the list container
/// itself does not necessarily report `is_focus` even though the keyboard
/// genuinely reached it.
fn list_has_keyboard(window: &Window) -> bool {
    let list = window.list();
    list.is_focus() || list.focus_child().is_some()
}

pub fn return_and_tab_move_the_keyboard_to_the_message_list() {
    // A guarded temporary, as every other case here uses. This built its own
    // path under `temp_dir()` and never removed it, so each run left a
    // `postio-search-return-tab-<pid>` directory behind for ever — two of
    // them were already on this machine when #1034 was picked up.
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
    let report = seed_small(&database, 11);
    assert!(
        report.message_count > 0,
        "the fixture seeded no mail, so this test could not fail"
    );
    ensure_search_index(&database).expect("the index is part of opening the store");
    let directory = tempfile::tempdir().expect("a blob directory");
    // `directory.path()`, not `directory.keep()`: `keep` consumes the guard
    // and leaks the directory, which is one way to stop the store's files
    // vanishing underneath it and the only one here that never cleans up.
    // Holding the guard to the end of the test does the same job.
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    feed_the_window(&window, &wiring).expect("the store has an account");

    let finder = window.finder();

    // ── Return runs the search and leaves the field ──────────────────────
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    let live = finder
        .live()
        .expect("the box has a live readout while searching");

    finder.activate();
    let answered = settle_until(|| live.outcome().is_some());
    assert!(
        answered,
        "Return never produced a readout -- this test is not exercising a \
         real search"
    );
    assert!(
        !finder.has_keyboard(),
        "Return ran the search but left the keyboard in the field"
    );
    assert!(
        list_has_keyboard(&window),
        "Return ran the search but did not move the keyboard to the \
         message list"
    );

    // ── Tab with nothing to refine also reaches the list ──────────────────
    // A query nothing in the corpus matches has no result set to draw
    // refinements from, so `focus_refine` has no chip to hand the keyboard
    // to and this is the fallback path -- deterministically, unlike waiting
    // out the facets round trip on a query that does match (`search_wiring.rs`
    // confirms QUERY itself offers chips, which would claim Tab first).
    const NO_HITS: &str = "zzz-nothing-in-the-corpus-matches-this-693";
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: NO_HITS.to_owned(),
    });
    settle_until(|| {
        finder
            .live()
            .is_some_and(|live| live.outcome().is_some_and(|outcome| outcome.hits == 0))
    });

    let claimed = finder.press_tab();
    assert!(
        claimed,
        "Tab with no refine chips available did nothing -- it must claim \
         the keyboard itself rather than falling through to an \
         unpredictable GTK focus-chain destination"
    );
    assert!(
        list_has_keyboard(&window),
        "Tab claimed the keyboard but did not move it to the message list"
    );

    bridge.shutdown();
    // Held to here on purpose: dropping either earlier removes a directory
    // the store and the state file are still using.
    drop(directory);
    drop(state_dir);
}
