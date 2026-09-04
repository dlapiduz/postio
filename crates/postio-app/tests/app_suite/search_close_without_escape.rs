//! Leaving search by a path other than a literal `Escape` keypress still
//! puts the folder back (#1011).
//!
//! `Feed::close_results` exists specifically to hand the list back to a
//! mailbox instead of search hits, and `search_results.rs` already proves the
//! one path that reaches it end to end: `Finder::press_escape` — what
//! `finder.rs`'s own key controller calls while the search entry itself has
//! the keyboard — fires `on_dismissed`, and `postio-app::search::install`'s
//! handler on that signal calls `close_results` and restores the folder.
//!
//! That is not the only way `Window` closes the finder. `CommandId::Back`
//! (what `Esc` becomes once GTK's window-level shortcut controller answers
//! it — the case once the keyboard has moved off the search entry onto the
//! list to read a result) and four other places in `window.rs` call
//! `Window::close_finder()` **directly**, never touching `Finder::dismiss`
//! at all. `close_finder` restores keyboard focus and the keymap context —
//! it says nothing about `Feed` — so every one of those paths left the list
//! showing stale hits with no query box open to explain why. The report was
//! exact: search, read a result with the keyboard on the list, dismiss —
//! and the list holds a folder open again only after clicking it by hand.
//!
//! So this drives `Command::Back` -- through `Window::act`, the same seam
//! every other command test in this suite uses -- rather than
//! `press_escape()`, which is the one path `search_results.rs` never
//! exercises and the one this bug lived behind.
//!
//! One test function, for the reason `search_open.rs` gives: GTK is
//! single-threaded and both the search and the page reads answer on the
//! thread-default main context.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::MessageId;
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A word every fixture in the corpus carries in a header, so the query is
/// about the wiring rather than about the corpus.
const QUERY: &str = "example.com";

/// Every id the list model is currently holding, in list order.
fn listed(list: &postio_gtk::list::MessageList, total: u32) -> Vec<MessageId> {
    (0..total)
        .filter_map(|position| list.peek(position))
        .collect()
}

pub fn closing_the_finder_without_pressing_escape_still_restores_the_folder() {
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
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let wired = feed_the_window(&window, &wiring).expect("the store has an account");
    let feeds = wired.feeds.clone();

    let notifier = postio_app::notifications::Notifier::new(
        database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    for stream in [events, replies] {
        commands::drain(&window, &feeds, stream, notifier.clone());
    }

    // ── the list starts on the folder ───────────────────────────────────
    let list = window.list().model();
    let filled = settle_until(|| !listed(&list, 1).is_empty());
    assert!(
        filled,
        "the list never showed the mailbox, so this test cannot tell a \
         search that failed to reach it from a list that was never fed"
    );
    let mailbox_rows = listed(&list, report.message_count as u32);
    let mailbox_name = window.list().mailbox_name();

    // ── search, and let the hits reach the list ───────────────────────────
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    finder
        .live()
        .expect("the box has a live readout while searching")
        .flush();

    let switched = settle_until(|| feeds.messages.showing_results());
    assert!(
        switched,
        "the box never switched the list to results, so this test cannot \
         tell a close that fails to restore the folder from one that never \
         had anything to restore"
    );

    // ── the gesture: `Back` while the finder is open ─────────────────────
    //
    // `press_escape()` is what `search_results.rs` already exercises, and it
    // works -- that is what `finder.rs`'s own key controller calls while the
    // search entry itself has the keyboard. `Command::Back` is the other
    // door: what `Esc` becomes once GTK's window-level shortcut controller
    // answers it instead, which is what happens once the keyboard has moved
    // off the search entry onto the list to read a result -- reaching
    // `Window::handled_here`'s own `CommandId::Back` arm rather than
    // `Finder::dismiss` at all.
    assert!(finder.is_open(), "the finder has to be open to close");
    window.act(postio_core::Command::Back);

    let left = settle_until(|| !feeds.messages.showing_results());
    assert!(
        left,
        "`Back` while the finder was open left the list showing search \
         hits. `Window::handled_here`'s `CommandId::Back` arm calls \
         `close_finder`, which restores keyboard focus and the keymap \
         context but never told `Feed` the search was over, so the mailbox \
         never came back -- exactly the report: the list stays on stale \
         results until a folder is clicked by hand."
    );
    let back = settle_until(|| listed(&list, 1) == mailbox_rows[..1]);
    assert!(
        back,
        "the list came back from the search showing something other than \
         the folder it went in on"
    );
    assert_eq!(
        window.list().mailbox_name(),
        mailbox_name,
        "the column header is still counting results over a folder listing"
    );

    bridge.shutdown();
}
