//! `AppState::set_connection` had no production caller (#974): nothing wired
//! an incoming `Event::ConnectionChanged` into it, so `AppState::accounts()`
//! and `AppState::connection` answered as though no account existed however
//! many were configured and however they stood with their servers.
//!
//! Three criteria, one window, the reason `wiring.rs` gives:
//!
//! 1. a connection event reaching the bus makes `AppState::accounts()`
//!    non-empty;
//! 2. `g a` (`CommandId::NextScope`) cycles the sidebar's scope with two
//!    accounts configured — proved through `Window::act`, the same door the
//!    keybinding uses, not by calling the widget method directly;
//! 3. `AppState::connection` and the frontend's own `Trackers`
//!    (`Feeds::folders`) agree about an account that has gone offline.
//!
//! `g a` is answered entirely inside `Window::act` — `Sidebar::select_next_scope`
//! walks the strip's own rows and never reaches `postio_core::state::AppState::next_scope`,
//! which has no caller anywhere in the workspace outside its own unit tests.
//! Criterion 2 is proved through the mechanism that actually runs today
//! rather than through the dead one the issue's own text names, and the
//! stale doc comment on `next_scope` is corrected alongside this file.
//!
//! Nothing here touches the network: `feed_the_window` reads the local store
//! and the connection events are delivered by hand, the way the runtime
//! would deliver them.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub};
use postio_core::state::SharedState;
use postio_core::{Command, ConnectionState, Event};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, test_support};

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

pub fn a_connection_event_a_scope_cycle_and_the_trackers_all_agree_with_appstate() {
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

    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "the window drew no mail at all, so nothing below can be concluded"
    );
    assert!(
        settle_until(|| window.sidebar().account_names().len() == 2),
        "the fixture seeded two accounts and the strip does not show them"
    );
    // What criterion 2 below actually observes: the strip's own scope
    // selection, the same signal `connect_scope_selected`'s real listener
    // (re-pointing the folder tree and the list) reacts to. Recording it
    // here rather than polling `feeds.messages.scope()` keeps the assertion
    // about *cycling*, not about how long a folder-tree reload after an
    // account switch takes to settle -- a separate, slower concern
    // `gtk_folder_sections.rs` already covers.
    let picked: std::rc::Rc<std::cell::RefCell<Vec<postio_model::AccountScope>>> =
        std::rc::Rc::default();
    window.sidebar().connect_scope_selected({
        let picked = std::rc::Rc::clone(&picked);
        move |scope| picked.borrow_mut().push(scope)
    });

    // ── 1. a connection event reaching the bus fills AppState::accounts() ─
    assert!(
        state.read(|app_state| app_state.accounts().is_empty()),
        "nothing has reported yet; if this is already non-empty the test \
         below proves nothing about the wiring"
    );
    engine.emit(Event::ConnectionChanged {
        account: first.account.id,
        state: ConnectionState::Online,
    });
    assert!(
        settle_until(|| state.read(|app_state| !app_state.accounts().is_empty())),
        "AppState::accounts() is still empty after a connection event reached \
         the bus -- set_connection has no caller"
    );
    assert!(
        state.read(|app_state| app_state.accounts().contains(&first.account.id)),
        "the account the event named should be the one AppState now knows about"
    );

    // ── 2. `g a` cycles the scope through `Window::act`, the keybinding's
    //       own door ──────────────────────────────────────────────────────
    window.act(Command::NextScope);
    settle();
    window.act(Command::NextScope);
    settle();
    let seen = picked.borrow().clone();
    assert_eq!(
        seen.len(),
        2,
        "two g a presses should have picked two rows through Window::act, \
         the same door the keybinding uses: {seen:?}"
    );
    assert_ne!(
        seen[0], seen[1],
        "the second g a landed on the same scope as the first, so nothing \
         actually cycled: {seen:?}"
    );

    // ── 3. AppState::connection agrees with the frontend's own Trackers ──
    engine.emit(Event::ConnectionChanged {
        account: second.account.id,
        state: ConnectionState::Offline,
    });
    assert!(
        settle_until(
            || state.read(|app_state| app_state.connection(second.account.id))
                == ConnectionState::Offline
        ),
        "AppState never heard the account went offline"
    );
    let from_state = state.read(|app_state| app_state.connection(second.account.id));
    let from_trackers = feeds
        .folders
        .statuses()
        .into_iter()
        .find(|(id, _)| *id == second.account.id)
        .map(|(_, status)| status.state);
    assert_eq!(
        Some(from_state),
        from_trackers,
        "AppState and the frontend's Trackers disagree about the same account: \
         AppState says {from_state:?}, Trackers says {from_trackers:?}"
    );

    bridge.shutdown();
}
