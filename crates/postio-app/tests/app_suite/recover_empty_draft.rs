//! An untouched compose buffer is not recovered after a crash.
//!
//! #491 reopens whatever draft was mid-edit when the last session died, and
//! that is right for a draft with something in it: "a draft is not durable
//! until it comes back on its own". It is wrong for a draft with nothing in
//! it, and worse than wrong -- it is self-perpetuating. Recovery opens the
//! composer, the composer autosaves an `Editing` row for the empty buffer it
//! is holding, the next unclean stop recovers *that*, and the client opens
//! into a stale compose buffer every launch until somebody types into it and
//! sends. Which is the exact symptom #491's own doc names as reading broken.
//!
//! Reported from a live run: the app opened on the composer, and `/` did
//! nothing because in the composer `/` is a character somebody is typing.
//!
//! `composer::closing` already answers "is there anything here worth
//! keeping" -- it is what decides whether Esc parks a draft or drops it --
//! so recovery asks it rather than inventing a second definition of empty.
//! Whitespace and the signature do not count, per that function's own rule.
//!
//! The crash is simulated the only way it can be: `begin_session` flips the
//! marker to `open` and reports what it found, so calling it once here leaves
//! the store looking exactly like a process that died mid-session, and
//! `compose::install`'s own call is then the one that sees a crash.
//!
//! Nothing here touches the network. One test function: GTK is
//! single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::{gdk, glib};
use postio_app::{Wiring, actions, commands, compose, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::Draft;
use postio_storage::repository::DraftRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn an_untouched_draft_is_not_recovered_into_the_composer() {
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
    let report = seed_small(&database, 9);
    let account = report.account.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A draft exactly as it opened: no recipient, no subject, no body. This
    // is what a recovered-then-killed composer leaves behind, and what every
    // launch after the first one was finding.
    {
        let connection = database.connection().expect("a connection");
        let mut draft = Draft::new(account);
        DraftRepository::new(&connection)
            .save(&mut draft)
            .expect("save the draft");
    }

    // The crash. After this the marker says `open`, so the `begin_session`
    // inside `compose::install` reports one.
    postio_session::begin_session(&database);

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);
    compose::install(
        &window,
        account,
        database.clone(),
        blobs,
        bridge.handle(),
        postio_app::reading::Showing::default(),
        {
            let feeds = feeds.clone();
            std::rc::Rc::new(move |event: &postio_core::Event| feeds.apply(event))
        },
    );
    while glib::MainContext::default().iteration(false) {}

    assert!(
        !window.composer().is_open(),
        "an untouched compose buffer is not work worth restoring, and \
         reopening it takes the keyboard from the inbox at every launch"
    );
}
