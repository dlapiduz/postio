//! A search that matched nothing says so, rather than saying the inbox is
//! empty.
//!
//! `list_state` has had a `NoMatches` state, its copy and its selection rule
//! since long before this. Its own doc says why it is separate: the mailbox
//! is not empty, the query is, and telling somebody who searched for an
//! invoice that they have nothing left to triage is a different statement and
//! a false one.
//!
//! What was missing was anything *saying* a search was on.
//! `Window::set_searching` existed with no caller anywhere in the workspace,
//! so the list never knew, and a search that found nothing drew the inbox's
//! own empty state -- "Nothing left to triage. 0 messages still in the local
//! store. Never synced yet." -- beside a mailbox holding thousands.
//!
//! It is asserted here rather than in `postio-gtk` because the missing wire
//! is here. Every layer below it passes on its own, which is exactly why
//! nothing caught this: `check-uncalled-pub-fn.py` cannot see it either,
//! since `postio-gtk` is one of its excluded frontends.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::{Wired, commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub, handler_fn};
use postio_core::state::SharedState;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn a_search_with_no_hits_says_so_rather_than_naming_the_inbox() {
    crate::gtk_case(async {
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

        let database = test_support::memory().await;
        let report = seed_small(&database, 11).await;
        assert!(report.message_count > 0, "the fixture seeded no mail");
        ensure_search_index(&database)
            .await
            .expect("the index is part of opening the store");
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        // A real bridge, so a command that nothing answers is rejected by the
        // real dispatcher rather than swallowed by a stub that accepts anything.
        //
        // And `run`'s own event arrangement, because the results only reach the
        // list through `Event::SearchResults` -> `Feeds::apply`: one hub the
        // search emits into, drained into the panes. Without the drain the box
        // finds hits, the preview shows one, and the list never leaves the
        // folder -- so there would be nothing for the open to land on, and the
        // test would fail for a reason that is not the bug.
        let hub = EventHub::new();
        let engine = hub.sink();
        let bridge = Bridge::builder()
            .build_with_events(handler_fn(|_, _| async {}), hub.sink())
            .expect("a runtime");
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            engine,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}

        // The same call `run` makes, and the `View` it returns rather than a
        // second `search::install` — two installs answer into a view the test
        // cannot see.
        let Wired { feeds, search } = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        // The view is installed the way `run` installs it; this case reads the
        // list's state rather than the column, so it is not held.
        let _view = search.expect("search installed");
        let notifier = notifications::Notifier::new(
            wiring.database.clone(),
            wiring.store.clone(),
            wiring.runtime.clone(),
            Default::default(),
        );
        commands::drain(
            &window,
            &feeds,
            hub.subscribe("window"),
            notifier,
            SharedState::default(),
        );

        // ── search for something the store certainly does not hold ───────────
        let finder = window.finder();
        finder.open(Mode::Search);
        finder.set_query(Query {
            mode: Mode::Search,
            text: "xylophone".to_owned(),
        });
        finder
            .live()
            .expect("the box has a live readout while searching")
            .flush();

        let said = settle_until(async || {
            matches!(
                window.list_state().state(),
                Some(postio_gtk::list_state::State::NoMatches { .. })
            )
        })
        .await;
        assert!(
            said,
            "a search that found nothing is showing {:?} -- a statement about the \
         mailbox, where the question was about the query",
            window.list_state().state()
        );
    })
}
