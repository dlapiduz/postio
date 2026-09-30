#![allow(unsafe_code)]
//! Archiving a conversation on screen takes its row out where it stands
//! (#1607).
//!
//! An archive reloaded the list: every row's widget rebuilt, every seek mark
//! dropped, page 0 read again -- to take out one row the list was holding.
//! This runs the real chain: the key, the bus over the local store, the
//! events it emits routed into the feeds the way the application routes
//! them, and the list the person is looking at.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, actions};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn archiving_a_conversation_on_screen_takes_out_only_its_row() {
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
        let (bridge, events) = Bridge::new(bus).expect("a runtime");
        let (sink, _unused) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let feeds = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account")
            .feeds;
        commands::install(&window, &feeds, state, wiring.commands.clone(), wired);
        glib::spawn_future_local({
            let feeds = feeds.clone();
            async move {
                while let Some(event) = events.next().await {
                    feeds.apply(&event);
                }
            }
        });

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 2).await,
            "no rows to archive"
        );
        // A conversation of one: archiving it takes the whole conversation
        // out of the folder, so its row has nowhere else to be drawn from.
        let model = list.model();
        let lone = (0..model.n_items().saturating_sub(1)).find_map(|index| {
            let row = model
                .item(index)?
                .downcast::<postio_gtk::list::MessageRow>()
                .ok()?
                .row()?;
            (row.thread_count == 1).then_some(index)
        });
        let Some(position) = lone else {
            panic!("the seeded folder has no conversation of one message to archive");
        };
        let next = model
            .peek(position + 1)
            .expect("the row after it is on screen");
        let archived = model.peek(position).expect("the row is on screen");
        list.select_message(archived);
        for _ in 0..20 {
            while glib::MainContext::default().iteration(false) {}
        }

        let rows = model.n_items();
        let pages = postio_ui::test_support::pages_requested();
        window.handle_key(gdk::Key::a, gdk::ModifierType::empty());
        assert!(
            settle_until(async || model.n_items() == rows - 1).await,
            "archiving took no row out: {} rows",
            model.n_items()
        );
        for _ in 0..40 {
            while glib::MainContext::default().iteration(false) {}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            model.peek(position),
            Some(next),
            "the row after the archived one moved up into its place"
        );
        let read = postio_ui::test_support::pages_requested() - pages;
        assert_eq!(
            read, 0,
            "taking out a row the list held read {read} pages to do it"
        );
        bridge.shutdown();
    });
}
