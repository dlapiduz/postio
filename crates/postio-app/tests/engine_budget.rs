//! More accounts than the pool can serve is a sentence, not a hang (#183).
//!
//! ADR 0005 Q3: each engine holds a connection from the pool for the length
//! of a sync pass, so an engine the pool cannot serve deadlocks waiting for a
//! connection another pass is holding — and a deadlock says nothing. The
//! composition root refuses up front instead, with a sentence on the event
//! stream, and starts *no* engine: starting some and not others would sync
//! whichever came first and silently skip the rest.
//!
//! `open_store` sizes the pool from the account count, so in a shipping
//! session this guard fires only when accounts were added after the store was
//! opened. The test recreates exactly that: a pool opened for one account, a
//! store that now holds three.
//!
//! # Why the fixture's hosts are 127.0.0.1
//!
//! On the guarded path nothing dials — the refusal returns before any engine
//! exists. The hosts matter only if the guard *regresses*: `engine::start`
//! wires a real transport and brings the link up, so a broken guard with
//! `imap.example.com` fixtures would put DNS lookups in the default suite.
//! Pointing at a closed local port turns that failure mode into a fast
//! connection-refused instead of a network violation.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. Set before the app under test starts.

use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_app::start_syncing;
use postio_core::Event;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::secret::MemorySecretStore;
use postio_model::{Account, EmailAddress};
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};

fn local_account(connection: &postio_storage::PooledConnection, name: &str, address: &str) {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.incoming.host = "127.0.0.1".to_owned();
    account.incoming.port = 1;
    account.outgoing.host = "127.0.0.1".to_owned();
    account.outgoing.port = 1;
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("an account");
}

#[test]
fn too_many_accounts_for_the_pool_is_a_sentence_not_a_hang() {
    let state_dir = std::env::temp_dir().join(format!("postio-budget-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // A pool the size `open_store` would pick for one account, over a store
    // that holds three -- the "accounts added after open" state.
    let database = test_support::memory_with(postio_session::pool_size_for(1));
    let connection = database.connection().expect("a connection");
    local_account(&connection, "Ada", "ada@example.com");
    local_account(&connection, "Grace", "grace@example.net");
    local_account(&connection, "Lena", "lena@example.org");
    drop(connection);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands())
        .with_secrets(Arc::new(MemorySecretStore::new()));

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    start_syncing(&window, &wiring);

    assert_eq!(
        wiring.engine.count(),
        0,
        "engines were started past the pool's budget, which is the deadlock \
         this guard exists to refuse"
    );
    let error = std::iter::from_fn(|| events.try_next()).find_map(|event| match event {
        Event::Error { message } => Some(message),
        _ => None,
    });
    let Some(sentence) = error else {
        panic!("the refusal produced no sentence, so the user sees a hang's silence");
    };
    assert!(
        sentence.contains('3') && sentence.contains('1'),
        "the sentence names neither the account count nor the budget: {sentence}"
    );

    bridge.shutdown();
}
