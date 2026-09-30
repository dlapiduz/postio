//! A word that found nothing lists the mail for the word that was meant, and
//! the column says so (ADR 0037, amended).
//!
//! Every layer of this is tested on its own: the index offers the word
//! (`postio-index`), the box's search reruns with it (`postio-session`), the
//! column draws the line (`postio-gtk`). What only the composition root can
//! get wrong is the join -- results that arrive rewritten and a column that
//! is never told, so the list silently answers a word nobody typed.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wired, commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub, handler_fn};
use postio_core::state::SharedState;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::sql::RowExt as _;
use postio_storage::{BlobStore, test_support};

pub fn a_misspelled_word_lists_the_mail_it_meant_and_says_so() {
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
        // Held: this case reads the column, where the rewrite is said.
        let view = search.expect("search installed");
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

        // ── a word the store holds, misspelled ───────────────────────────────
        // Read off the store rather than named here, so the case does not
        // depend on which fixtures the seed happens to draw: the longest
        // plain word in any subject, with its second letter dropped.
        let word = {
            let connection = database.connect().await.expect("checkout");
            let subjects: Vec<String> = postio_storage::sql::all(
                &connection,
                "SELECT subject FROM messages WHERE subject IS NOT NULL",
                (),
                |row| row.col(0),
            )
            .await
            .expect("subjects");
            subjects
                .iter()
                .flat_map(|subject| subject.split(|c: char| !c.is_alphabetic()))
                .filter(|word| word.len() >= 8 && word.is_ascii())
                .max_by_key(|word| word.len())
                .expect("the seed has a subject with a long word")
                .to_lowercase()
        };
        let typo = format!("{}{}", &word[..1], &word[2..]);

        let finder = window.finder();
        finder.open(Mode::Search);
        finder.set_query(Query {
            mode: Mode::Search,
            text: typo.clone(),
        });
        finder
            .live()
            .expect("the box has a live readout while searching")
            .flush();

        let expected = format!("Showing results for {word}");
        let said = settle_until(async || said_in(view.panel().upcast_ref(), &expected)).await;
        assert!(
            said,
            "searching `{typo}` should list the mail for `{word}` and say so; the \
             list is showing {:?}",
            window.list_state().state()
        );
        assert!(
            !matches!(
                window.list_state().state(),
                Some(postio_gtk::list_state::State::NoMatches { .. })
            ),
            "the list holds the mail for the word that was meant"
        );
        assert_eq!(finder.query().text, typo, "the box keeps what was typed");
    })
}

/// Whether a mapped label under `widget` reads exactly `text`.
fn said_in(widget: &gtk::Widget, text: &str) -> bool {
    if widget.is_mapped()
        && widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == text)
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if said_in(&current, text) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}
