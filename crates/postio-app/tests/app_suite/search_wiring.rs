//! Search, driven the way a person drives it.
//!
//! `postio-1ag`. E9 built the whole search surface — the box's live readout,
//! the scope column, the refine chips, the preview — tested every one of them
//! against inputs it was handed, and shipped an application in which typing
//! into the box did nothing at all. The executor could not even be reached
//! from here until `postio-svx` split it out, and the index did not exist on
//! a real store until `postio-x4e` created it.
//!
//! So this asserts the far end, the way `wiring.rs` does: a store with mail in
//! it, a real window, the same `install` the binary calls, and then *type*.
//! Never "the readout renders an outcome it was given" — that has passed all
//! along — but "the readout has a number, and the number came from the store".
//!
//! One test function: GTK is single-threaded, and the search answers on the
//! thread-default main context, which the harness would otherwise drive from
//! two threads at once.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_search::facets::Scope;
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// A word every fixture in the corpus carries in a header, so the query is
/// about the wiring rather than about the corpus.
const QUERY: &str = "example.com";

/// Run the main loop until `done`, or give up.
///
/// A search crosses to the runtime and answers over a channel, so nothing has
/// happened the instant `flush` returns. A deadline rather than a spin count:
/// what is being waited for is a round trip.
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

pub fn typing_in_the_box_searches_the_store_and_fills_every_search_surface() {
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

    // ── a store the application has opened ──────────────────────────────
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(
        report.message_count > 0,
        "the fixture seeded no mail, so this test could not fail"
    );
    ensure_search_index(&database).expect("the index is part of opening the store");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ───────────────────────────────────────
    //
    // Through `feed_the_window`, because that is what makes it: search is
    // installed against the `Feeds` that owns the message list, so the hits
    // have somewhere to land. `search_results.rs` is what asserts they get
    // there; this stays about the surfaces around the list.
    //
    // The `View` comes back from that same call rather than from a second
    // `search::install`. Two installs put two handlers on the box, and the
    // query answers into the one a test cannot see.
    let view = feed_the_window(&window, &wiring)
        .expect("the store has an account")
        .search
        .expect("search installed");

    // ── type ────────────────────────────────────────────────────────────
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    let live = finder
        .live()
        .expect("the box has a live readout while searching");
    // What Enter does. A test that slept out the debounce would be measuring
    // the timer rather than the wiring.
    live.flush();

    let answered = settle_until(|| live.outcome().is_some());
    let outcome = live.outcome();
    assert!(
        answered,
        "typing `{QUERY}` into the box never produced a readout at all. The \
         box paces the query and hands it to whoever owns the store; if \
         nothing answered `Live::connect_run`, this is postio-1ag exactly — \
         every layer under it passes and the feature does not exist."
    );
    let outcome = outcome.expect("answered");

    // ── 1. a real hit count and timing ──────────────────────────────────
    assert!(
        outcome.hits > 0,
        "the store holds {} messages, every one of them from an {QUERY} \
         address, and the box reports {} hits. A readout wired to an executor \
         that is not reading this store looks exactly like this.",
        report.message_count,
        outcome.hits
    );

    // ── 2. the scope column has real counts, and switching rescopes ─────
    let panel = view.panel();
    assert_eq!(
        panel.scope(),
        Scope::AllMail,
        "the column starts on All Mail"
    );
    panel.set_scope(Scope::Inbox);
    let rescoped = settle_until(|| {
        live.outcome()
            .is_some_and(|later| later.hits != outcome.hits || panel.scope() == Scope::Inbox)
    });
    assert!(
        rescoped,
        "switching the scope column did not ask the question again. The scope \
         is state the panel owns and the query never carries, so nothing else \
         can notice it moved."
    );

    // ── 3. the refine chips reflect the real result set ─────────────────
    panel.set_scope(Scope::AllMail);
    let offered = settle_until(|| !view.panel().offered().is_empty());
    assert!(
        offered,
        "the refine column offers nothing for a query that matched {} \
         messages. Facets are a second read against the same result set; if \
         nobody runs it, the column stays empty however good the search was.",
        outcome.hits
    );

    // ── 4. the preview shows the focused result ─────────────────────────
    let focused = settle_until(|| view.preview().focused().is_some());
    assert!(
        focused,
        "the search found {} messages and the preview is showing none of \
         them. `View::set_focused` is what puts a hit in the pane, and \
         nothing calls it on its own.",
        outcome.hits
    );

    // ── 5. `@` lists real correspondents ────────────────────────────────
    finder.set_query(Query {
        mode: Mode::Contact,
        text: String::new(),
    });
    let correspondents = settle_until(|| !finder.matched_contacts().is_empty());
    assert!(
        correspondents,
        "`@` lists no correspondents over a store whose every message has a \
         sender. The finder matches over the whole list it was given, so an \
         empty result here means it was given nothing."
    );

    bridge.shutdown();
}
