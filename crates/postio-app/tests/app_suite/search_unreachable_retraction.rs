//! An account dropping out while a unified search is on screen updates the
//! caveat in place, without the query being asked again (#1060).
//!
//! `unified_search_reach.rs` proves the caveat attaches and retracts at all
//! — but it does so through `ask_again`, which reruns the search by hand.
//! That is exactly the gap #1060 names: the caveat used to be fixed at the
//! moment a search's answer came back, so an account that dropped out or
//! came back while the result sat on screen left the readout saying
//! something that had stopped being true, until somebody typed the query
//! again. This proves the real path — `commands::drain`'s event loop, the
//! one a running application actually uses — updates the caveat by itself.
//!
//! Nothing here touches the network: the connection events are delivered by
//! hand, the way the runtime would deliver them.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub};
use postio_core::state::SharedState;
use postio_core::{ConnectionState, Event};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::search::Outcome;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, test_support};

/// A word every fixture in the corpus supplies, so both accounts have hits
/// and the caveat is about reach rather than about an empty answer.
const QUERY: &str = "example.com";

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn outcome(window: &Window) -> Option<Outcome> {
    window
        .finder()
        .live()
        .expect("the box has a live readout while searching")
        .outcome()
}

pub fn an_account_going_away_and_coming_back_updates_the_caveat_without_asking_again() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let first = seed_small(&database, 11);
    let second = seed_extra_account(&database, "Second", "grace@example.org", 12);
    ensure_search_index(&database).expect("the index is part of opening the store");

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // ── `run`'s own arrangement: one hub, one subscription ──────────────
    let state = SharedState::default();
    let bus = postio_app::actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<postio_core::CommandId> = bus.wired().collect();
    let hub = EventHub::new();
    let engine = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(bus, hub.sink())
        .expect("a runtime");
    let wiring = Wiring::new(
        database,
        blobs,
        bridge.handle(),
        engine.clone(),
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        wired,
    );
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
        state.clone(),
    );

    // Every account reports in first. A tracker that has heard nothing is
    // *offline*, so without this every assertion below would be true for
    // the wrong reason.
    for account in [first.account.id, second.account.id] {
        engine.emit(Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
    }
    settle();

    // How many times the store was actually asked. #1060's whole point is
    // that a caveat updating in place costs none of these.
    let asked: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let finder = window.finder();
    let live = finder
        .live()
        .expect("the box has a live readout while searching")
        .clone();
    live.connect_run({
        let asked = asked.clone();
        move |_query, _sequence| asked.set(asked.get() + 1)
    });

    // ── a unified search, everything answering ───────────────────────────
    window.sidebar().test_click_account_row(0);
    assert!(
        settle_until(|| window.scope() == postio_core::Scope::Unified),
        "clicking the Unified row did not put the window in the unified scope"
    );
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    live.flush();
    assert!(
        settle_until(|| outcome(&window).is_some()),
        "the search never answered at all"
    );
    assert_eq!(
        outcome(&window).map(|outcome| outcome.unreachable),
        Some(Vec::new()),
        "every account reported online, so a unified search carries no caveat"
    );
    let reached_everything = outcome(&window).expect("an outcome").hits;
    assert!(
        reached_everything > 0,
        "the seeded accounts have no mail matching {QUERY:?}"
    );
    let asked_before = asked.get();
    assert!(asked_before > 0, "the search above should have asked once");

    // ── the second account drops out while the answer is on screen ───────
    //
    // No `ask_again` here: the whole point is that nobody types the query
    // over. If the caveat only attached because a rerun happened, this
    // reproduces exactly nothing.
    engine.emit(Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Offline,
    });
    let attached = settle_until(|| {
        outcome(&window).is_some_and(|outcome| outcome.unreachable == vec!["Second".to_owned()])
    });
    assert!(
        attached,
        "an account went offline while the result sat on screen and the \
         readout did not say so: {:?}",
        outcome(&window).map(|outcome| outcome.unreachable)
    );
    assert_eq!(
        outcome(&window).expect("an outcome").hits,
        reached_everything,
        "attaching the caveat must not change the hit count"
    );
    assert_eq!(
        asked.get(),
        asked_before,
        "attaching the caveat must not ask the store anything"
    );

    // ── and it comes back ─────────────────────────────────────────────────
    engine.emit(Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Online,
    });
    let retracted =
        settle_until(|| outcome(&window).is_some_and(|outcome| outcome.unreachable.is_empty()));
    assert!(
        retracted,
        "the account came back and the readout still names it: {:?}",
        outcome(&window).map(|outcome| outcome.unreachable)
    );
    assert_eq!(
        outcome(&window).expect("an outcome").hits,
        reached_everything,
        "retracting the caveat must not change the hit count either"
    );
    assert_eq!(
        asked.get(),
        asked_before,
        "retracting the caveat must not ask the store anything either"
    );

    // No reference-cycle assertion here: #1060's own wiring lives entirely
    // inside `commands::apply`, reached only from `commands::drain`'s
    // existing weak-window loop (`commands.rs`), and stores nothing new on
    // `Folders` or `Window` — so by construction it cannot be the thing
    // keeping either alive. A window built for this test does not actually
    // drop on `destroy()` today regardless of this change (#1072 --
    // `install_leave_to_list` in this same file gives `Finder::connect_search`
    // a *strong* `window` clone, a pre-existing cycle this issue's own fix
    // does not touch), so a runtime "the window dropped" check would be
    // testing that gap, not this one.
    drop(live);
    drop(finder);
    window.destroy();
    settle();

    bridge.shutdown();
}
