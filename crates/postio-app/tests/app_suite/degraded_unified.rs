//! The unified list says which account it could not reach (#187, ADR 0005 Q10).
//!
//! The rule: *a view that cannot include an account says so, names the
//! account, and stays usable.* `list_state.rs`'s own tests prove
//! `derive_aggregate` picks the right state from the right inputs. This
//! proves the inputs arrive — that a connection event for an account which is
//! **not** the one in view reaches the pane at all, which is the half that
//! was broken: `Folders` folded every account's `ConnectionChanged` into one
//! tracker, so the state of the account being asked about was whatever had
//! reported most recently.
//!
//! It also covers the criterion that is easy to build and easy to forget —
//! the banner clearing itself when the account comes back. A disclosure that
//! does not retract is one people learn to ignore.
//!
//! Nothing here touches the network: `feed_the_window` reads the local store
//! and the connection events are delivered by hand, the way the runtime would
//! deliver them.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_core::{ConnectionState, Event};
use postio_gtk::list_state::State;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ListScope;
use postio_session::Wiring;
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, test_support};

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

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

pub fn the_unified_list_names_an_account_it_could_not_reach_and_then_forgets_it() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let first = seed_small(&database, 11);
    let second = seed_extra_account(&database, "Second", "grace@example.org", 12);

    let directory = tempfile::tempdir().expect("a blob directory");
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
    settle();

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the window drew no mail at all, so nothing below can be concluded"
    );

    // ── into the unified view ───────────────────────────────────────────
    window.sidebar().test_click_account_row(0);
    assert!(
        settle_until(|| feeds.messages.scope() == Some(ListScope::Unified)),
        "the unified scope was never reached, so this is #185's wiring \
         failing rather than anything about degradation"
    );

    // And wait for the unified page itself. The scope is set the moment the
    // row is clicked; the rows arrive a round trip later, and every
    // assertion below is about what the banner does *over rows*.
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the unified scope was reached and drew nothing at all"
    );

    // Before anything has reported: a tracker that has heard nothing is
    // offline, so a freshly-opened unified view is degraded and says so
    // about *both* accounts. Asserted because it is the cheapest possible
    // proof that the aggregate path runs at all -- if this is None, the pane
    // is still answering as a single account and nothing below can be read.
    assert_eq!(
        window.sidebar().account_names().len(),
        2,
        "the banner names accounts from the sidebar's list, and it is empty"
    );
    let fresh = settle_until(|| matches!(window.list_state().state(), Some(State::Partial { .. })));
    assert!(
        fresh,
        "nothing has reported yet, so every account is offline and the \
         unified pane should say so.\n  pane:     {:?}\n  scope:    {:?}\n  \
         statuses: {:?}\n  names:    {:?}",
        window.list_state().state(),
        feeds.messages.scope(),
        feeds.folders.statuses(),
        window.sidebar().account_names(),
    );

    // Both accounts report in. This is not scene-setting: a tracker that has
    // heard nothing is *offline* -- silence is not a claim that a server is
    // reachable -- so a view whose accounts have never reported is degraded,
    // correctly, and the interesting transition is the one away from healthy.
    for account in [first.account.id, second.account.id] {
        feeds.apply(&Event::ConnectionChanged {
            account,
            state: ConnectionState::Online,
        });
    }
    settle();
    assert!(
        !matches!(window.list_state().state(), Some(State::Partial { .. })),
        "every account reported online and the pane still claims one is away: {:?}",
        window.list_state().state()
    );

    // ── one account goes away ───────────────────────────────────────────
    //
    // Deliberately *not* the account the folder tree is pointed at: that is
    // the case a single shared tracker got wrong, because the event that
    // matters is about an account the sidebar is not currently showing.
    feeds.apply(&Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Offline,
    });
    settle();

    // Localised deliberately: if the per-account routing is right and the
    // pane is still silent, the fault is in what the window asks for rather
    // than in what the feed knows, and these two assertions fail apart.
    let seen = feeds.folders.statuses();
    assert!(
        seen.iter().any(
            |(id, status)| *id == second.account.id && status.state == ConnectionState::Offline
        ),
        "the feed did not record the second account as offline: {seen:?}"
    );
    assert_eq!(
        seen.len(),
        2,
        "the aggregate has to be told about every account it is drawing, or \
         the one that is away is not in the list to be named: {seen:?}"
    );

    let named = settle_until(|| {
        matches!(
            window.list_state().state(),
            Some(State::Partial { ref accounts }) if accounts == &vec!["Second".to_owned()]
        )
    });
    assert!(
        named,
        "an account went offline under a unified view and the pane says {:?}. \
         ADR 0005 Q10: the view names the account it cannot vouch for.",
        window.list_state().state()
    );

    // Usable, not blocked: the mail that did arrive is still on screen and
    // still scrollable underneath. A modal or a full plate here would keep
    // the promise in words and break it on the screen.
    assert!(
        list.model().n_items() > 0,
        "the banner replaced the rows instead of sitting over them"
    );
    assert!(
        window.list_state().is_visible(),
        "the state was derived and the widget never showed it"
    );

    // ── and it comes back ───────────────────────────────────────────────
    feeds.apply(&Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Online,
    });
    settle();

    let cleared =
        settle_until(|| !matches!(window.list_state().state(), Some(State::Partial { .. })));
    assert!(
        cleared,
        "the account recovered and the pane still says {:?}; a disclosure \
         that never retracts is one people learn to ignore",
        window.list_state().state()
    );

    bridge.shutdown();
}
