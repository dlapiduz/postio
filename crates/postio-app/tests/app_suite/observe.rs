//! Where everything is, as Classic reports it to a storyboard.
//!
//! `Window::observe` is what a storyboard's checks read after every step
//! (specs/008-storyboards contracts/observation.md). Every field it fills
//! must come from what is really on screen -- the widget that holds the
//! keyboard, the cursor's row, the toast that is up -- because a field read
//! from what a layer was *told* is exactly the kind of assertion that cannot
//! fail when the wiring between them breaks.
//!
//! So this drives the real composition root over a seeded store, delivers
//! keys along the real focus chain, and asks the window after each one.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window, notifications};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, EventHub};
use postio_core::state::SharedState;
use postio_gtk::storyboard::deliver;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, actions};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};
use postio_ui::keymap::Chord;
use postio_ui::observe::{Overlay, Region, Tone, View};

fn press(window: &Window, chord: &str) {
    let chord: Chord = chord.parse().expect("a chord");
    let delivery = deliver::press(window.upcast_ref(), &chord).expect("a deliverable chord");
    assert!(
        matches!(delivery, deliver::Delivery::Delivered { .. }),
        "`{chord:?}` was dropped on the way to the window"
    );
    while glib::MainContext::default().iteration(false) {}
}

pub fn the_window_says_where_the_keyboard_cursor_and_notices_are() {
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
        seed_small(&database, 11).await;
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        let state = SharedState::default();
        let bus = actions::wire(
            postio_core::dispatch::DispatcherBuilder::new(),
            actions::Actions::new(database.clone(), state.clone()),
        )
        .build();
        let wired: Vec<CommandId> = bus.wired().collect();
        // `run`'s own event arrangement: the bridge and the wiring both emit
        // into one hub, drained into the panes. Without the drain an archive
        // lands in the store and no toast ever says so.
        let hub = EventHub::new();
        let bridge = Bridge::builder()
            .build_with_events(bus, hub.sink())
            .expect("a runtime");
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            hub.sink(),
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let feeds = feed_the_window(&window, &wiring)
            .await
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
        commands::drain(&window, &feeds, hub.subscribe("window"), notifier, state);

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 1).await,
            "no rows to observe"
        );
        list.grab_focus();
        while glib::MainContext::default().iteration(false) {}

        // `j`: the list has the keyboard, and the cursor is on the second row.
        press(&window, "j");
        let seen = window.observe();
        assert_eq!(seen.keyboard.region, Region::List, "{seen:#?}");
        assert!(seen.keyboard.reachable);
        assert!(!seen.keyboard.typing);
        assert_eq!(seen.cursor.index, Some(1));
        assert!(seen.cursor.id.is_some());
        assert_eq!(seen.overlay.kind, Overlay::None);
        assert_eq!(seen.back_depth, None, "Classic has no back stack");
        assert!(
            seen.app.contains_key("classic.pane"),
            "{:?}",
            seen.app.keys()
        );

        // `/`: the search box is up and has the keyboard, and it takes text.
        press(&window, "/");
        let seen = window.observe();
        assert_eq!(seen.overlay.kind, Overlay::Finder, "{seen:#?}");
        assert_eq!(seen.keyboard.region, Region::Search);
        assert!(seen.keyboard.typing, "the search field takes text");

        // `Escape`: back to the list, the cursor where it was.
        press(&window, "Escape");
        let seen = window.observe();
        assert_eq!(seen.keyboard.region, Region::List, "{seen:#?}");
        assert_eq!(seen.overlay.kind, Overlay::None);
        assert_eq!(seen.cursor.index, Some(1));

        // `a`: the row goes, and the toast offers to take it back.
        press(&window, "a");
        assert!(
            settle_until(async || window.observe().notice.text.is_some()).await,
            "no notice after archive"
        );
        let seen = window.observe();
        assert!(seen.notice.undo, "{seen:#?}");
        assert_eq!(seen.notice.tone, Some(Tone::Info));
        // The reading pane follows the cursor, and shows a conversation by
        // default: the row that took the archived one's place is open in it.
        assert_eq!(seen.view, View::Conversation, "{seen:#?}");
        assert!(seen.reading.id.is_some(), "the pane names what it shows");
    });
}
