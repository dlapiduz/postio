//! Startup over an account whose password the keyring does not have.
//!
//! `postio-67`. Onboarding writes two things — the account row and the
//! credential — and 0.1.0 wrote them in that order, so a keyring write that
//! failed left the row behind. Startup then asked
//! `first_account(..).is_some()`, one row was enough, and every launch after
//! that opened an account that could not authenticate, could not sync, and
//! could not be repaired: onboarding is the only thing in the application
//! that writes a credential, and onboarding never ran again. Recovering
//! meant deleting rows from SQLite by hand.
//!
//! # Why this test is here and not in the crate's unit tests
//!
//! `startup_route` is unit-tested next to the code, and passing that proves
//! only that a *function* returns the right answer. The bug was never in a
//! function — it was in which branch the application took. So this drives
//! [`postio_app::open_or_onboard`], the same call `run()`'s `activate`
//! handler makes, over a real `Window`, and asserts on what the window is
//! showing afterwards. That is the only shape of assertion `postio-bl2` did
//! not survive.
//!
//! One test function, for the reason `wiring.rs` gives: GTK is initialised
//! once, per process, from one thread.
//!
//! Nothing here dials anything. The keyring is a `MemorySecretStore` and the
//! command handler is a no-op; `start_syncing` is the half that opens a
//! socket and `open_or_onboard` reaches it only down the branch this store
//! cannot take.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use adw::prelude::*;
use gtk::{gdk, glib};
use postio_app::notifications;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::secret::MemorySecretStore;
use postio_session::Wiring;
use postio_storage::{BlobStore, test_support};
use std::sync::Arc;

/// The onboarding screen, if that is what the window is showing.
fn screen(window: &Window) -> Option<Onboarding> {
    window.content().and_downcast::<Onboarding>()
}

pub fn an_account_with_no_credential_lands_on_the_repair_screen() {
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

    // ── the state 0.1.0 could get itself into ───────────────────────────
    // An account row, and a keyring that has nothing for it.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let account = test_support::account(&connection);
    drop(connection);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    )
    .with_secrets(Arc::new(MemorySecretStore::new()));

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        screen(&window).is_none(),
        "the window started on the onboarding screen, so this test cannot fail"
    );

    // ── the same call `run()`'s `activate` handler makes ────────────────
    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    postio_app::open_or_onboard(
        &window,
        &wiring,
        Default::default(),
        Vec::new(),
        std::rc::Rc::new(std::cell::RefCell::new(Some(events))),
        notifier,
        std::rc::Rc::new(std::cell::Cell::new(false)),
    );

    let arrived = settle_until(|| screen(&window).is_some());
    assert!(
        arrived,
        "an account with no password in the keyring opened as though it were \
         a working account. That is `postio-67`: the application has no other \
         way to write a credential, so this window is a dead end."
    );

    // Not merely *some* screen: a repair, which is a different thing from a
    // first run and has to read as one.
    let screen = screen(&window).expect("the onboarding screen");
    assert!(
        matches!(screen.status(), Status::Reauthenticate(_)),
        "the repair arrived looking like a first run: {:?}",
        screen.status()
    );
    assert_eq!(
        screen.address(),
        account.address.address,
        "the screen made the user retype an address the store already had"
    );
    assert_eq!(
        screen.settings().imap.host,
        account.incoming.host,
        "the screen made the user retype servers the store already had"
    );

    bridge.shutdown();
}
