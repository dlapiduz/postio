//! A unified search says which account it could not reach (#812, ADR 0005 Q10).
//!
//! The rule: *a view that cannot include an account says so, names the
//! account, and stays usable.* `degraded_unified.rs` proves the **list**
//! obeys it. A search is the other surface that can quietly answer for less
//! mail than the user thinks they asked about, and its answer is a number —
//! "no hits" from a complete search and "no hits" from one that could not
//! reach half the mail are the same three characters, and only one of them
//! means the mail is not there.
//!
//! The wording lives in `postio_gtk::search::readout` and is unit-tested
//! there. What this proves is the join: that the composition root notices an
//! account is away and puts it in the outcome the readout draws. Nothing here
//! touches the network — the connection events are delivered by hand, the way
//! the runtime would deliver them.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::{settle, settle_until};
use gtk::gdk;
use postio_app::{Wired, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
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

fn outcome(window: &Window) -> Option<Outcome> {
    window
        .finder()
        .live()
        .expect("the box has a live readout while searching")
        .outcome()
}

fn search_for(window: &Window, text: &str) {
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: text.to_owned(),
    });
    finder
        .live()
        .expect("the box has a live readout while searching")
        .flush();
}

/// Ask the query on screen again, which is what a person does when something
/// about their account has changed.
///
/// Not `search_for` with the same text: setting the query to what it already
/// is asks nothing, so the outcome on screen would still be the previous
/// one and every assertion after it would be about a stale answer. The
/// caveat is attached per search — a connection that drops while results are
/// on screen does not currently update it in place, which is #1060.
fn ask_again(window: &Window) {
    window
        .finder()
        .live()
        .expect("the box has a live readout while searching")
        .rerun();
}

pub fn a_unified_search_names_the_account_it_could_not_reach() {
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
    let first = seed_small(&database, 11);
    let second = seed_extra_account(&database, "Second", "grace@example.org", 12);
    ensure_search_index(&database).expect("the index is part of opening the store");

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    let Wired { feeds, .. } =
        feed_the_window(&window, &wiring).expect("the seeded store has an account");

    // Every account reports in first. A tracker that has heard nothing is
    // *offline* — silence is not a claim that a server is reachable — so
    // without this every assertion below would be true for the wrong reason.
    for account in [first.account.id, second.account.id] {
        feeds.apply(&Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
    }
    settle();

    // ── a single-account search is unchanged ─────────────────────────────
    search_for(&window, QUERY);
    assert!(
        settle_until(|| outcome(&window).is_some()),
        "the search never answered at all"
    );
    assert_eq!(
        outcome(&window).map(|outcome| outcome.unreachable),
        Some(Vec::new()),
        "a search of one account leaves nothing out, so it has nothing to \
         disclose — whatever the other account is doing"
    );
    let one_account = outcome(&window).expect("an outcome").hits;
    assert!(
        one_account > 0,
        "the seeded account has no mail matching {QUERY:?}"
    );

    // ── unified, everything answering ────────────────────────────────────
    window.sidebar().test_click_account_row(0);
    assert!(
        settle_until(|| window.scope() == postio_core::Scope::Unified),
        "clicking the Unified row did not put the window in the unified scope"
    );
    // Strictly more than one account's worth, which is what says the
    // *unified* answer has landed rather than the single-account one still
    // being on screen. Both accounts hold mail matching this query, so the
    // widening is the observable difference between the two searches.
    assert!(
        settle_until(|| outcome(&window).is_some_and(|outcome| outcome.hits > one_account)),
        "the unified search did not widen the answer ({:?} against {one_account} \
         for one account), so there is nothing here for a caveat to be \
         attached to",
        outcome(&window).map(|outcome| outcome.hits)
    );
    assert_eq!(
        outcome(&window).map(|outcome| outcome.unreachable),
        Some(Vec::new()),
        "every account reported online, so a unified search carries no \
         caveat. A disclosure that is always on is one people learn to ignore."
    );
    let reached_everything = outcome(&window).expect("an outcome").hits;

    // ── one account goes away ────────────────────────────────────────────
    //
    // Deliberately the account the folder tree is *not* pointed at: the
    // event that matters is about an account the sidebar is not showing,
    // which is the case a single shared tracker got wrong (#187).
    feeds.apply(&Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Offline,
    });
    settle();
    ask_again(&window);

    let named = settle_until(|| {
        outcome(&window).is_some_and(|outcome| outcome.unreachable == vec!["Second".to_owned()])
    });
    assert!(
        named,
        "an account went offline and a unified search still claims it \
         answered: {:?}. ADR 0005 Q10 — a view that cannot include an account \
         says so and names it — and a hit count is exactly the kind of answer \
         that looks complete when it is not.",
        outcome(&window).map(|outcome| outcome.unreachable)
    );
    assert!(
        postio_gtk::search::readout(&outcome(&window).expect("an outcome"))
            .contains("Second unreachable"),
        "the readout a person actually sees has to carry it: {}",
        postio_gtk::search::readout(&outcome(&window).expect("an outcome"))
    );

    // ── and a search that finds nothing says it too ──────────────────────
    //
    // The case Q10 singles out. "no hits" is the answer most likely to be
    // read as "that mail does not exist", and it is the one an unreachable
    // account is most likely to have caused.
    search_for(&window, "wordnofixturecarries");
    let empty_and_short = settle_until(|| {
        outcome(&window).is_some_and(|outcome| {
            outcome.hits == 0 && outcome.unreachable == vec!["Second".to_owned()]
        })
    });
    assert!(
        empty_and_short,
        "a unified search that matched nothing while an account was away \
         said nothing about it: {:?}",
        outcome(&window)
    );

    // ── the account comes back, and the caveat retracts ──────────────────
    //
    // A disclosure that does not retract is one people learn to ignore.
    feeds.apply(&Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Online,
    });
    settle();
    // A different query from the one on screen, so this genuinely re-runs:
    // the search before it asked for a word nothing carries, and asking that
    // again would settle on nought hits for a reason that has nothing to do
    // with reach.
    search_for(&window, QUERY);
    let cleared = settle_until(|| {
        outcome(&window).is_some_and(|outcome| {
            outcome.unreachable.is_empty() && outcome.hits == reached_everything
        })
    });
    assert!(
        cleared,
        "the account came back and the search still says it is away: {:?}",
        outcome(&window).map(|outcome| outcome.unreachable)
    );
}
