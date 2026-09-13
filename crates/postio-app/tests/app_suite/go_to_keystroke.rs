//! `g i` reaches the inbox, at the composition root.
//!
//! # Why here rather than beside the box
//!
//! `gtk_finder.rs` proves the box jumps to a folder by handing the finder a
//! fixture with `set_mailboxes` and watching a handler fire. That is an
//! assertion about what a layer was handed, and it would pass unchanged if
//! nothing in the running application ever fed it — which is the shape of
//! defect this codebase keeps producing, and the reason a folder jump that
//! had shipped could be requested as new work by the person who owns the
//! project.
//!
//! So this presses the key on a real window, wired the way `run` wires it,
//! and asks what folder a person would then be looking at.
//!
//! Nothing here touches the network: `start_syncing` is never called.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::MailboxRole;
use postio_session::{Wiring, actions};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// Presses a sequence like `g i`, one chord at a time.
fn press(window: &Window, keys: &[&str]) {
    for key in keys.iter().copied() {
        window.handle_key(
            gdk::Key::from_name(key).expect("a key by that name"),
            gdk::ModifierType::empty(),
        );
        while glib::MainContext::default().iteration(false) {}
    }
}

pub fn pressing_g_i_shows_the_inbox() {
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
        let report = seed_small(&database, 11).await;
        let inbox = report
            .mailbox(MailboxRole::Inbox)
            .expect("the fixture has an inbox")
            .id;
        let archive = report
            .mailbox(MailboxRole::Archive)
            .expect("the fixture has an archive")
            .id;
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

        let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
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
        while glib::MainContext::default().iteration(false) {}

        let feeds = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account")
            .feeds;
        commands::install(&window, &feeds, state, wiring.commands.clone(), wired);

        assert!(
            settle_until(async || window.sidebar().selected().is_some()).await,
            "the window never settled on a folder, so there is nothing to move away \
             from"
        );

        // Somewhere that is not the inbox, so arriving at it means something.
        window.open_mailbox(archive);
        assert!(
            settle_until(async || window.sidebar().selected() == Some(archive)).await,
            "the fixture could not be moved off the inbox"
        );

        // ── the keystroke ────────────────────────────────────────────────────
        press(&window, &["g", "i"]);

        assert!(
            settle_until(async || window.sidebar().selected() == Some(inbox)).await,
            "`g i` did not reach the inbox: the sidebar is still showing {:?}",
            window.sidebar().selected()
        );

        // ── `g` is a letter to somebody who is writing ───────────────────────
        // FR-042. `g` is one of the commonest letters in English prose, so a
        // sequence that fires while a draft is being typed would make the
        // composer unusable -- and it would do it by *navigating away*, which is
        // the worst available outcome for unsaved words.
        window.open_mailbox(archive);
        assert!(settle_until(async || window.sidebar().selected() == Some(archive)).await);

        press(&window, &["c"]);
        assert!(
            settle_until(async || window.composer().is_open()).await,
            "`c` did not open the composer, so nothing here would be typing into it \
             and a pass below would mean nothing"
        );
        press(&window, &["g", "i"]);

        assert_eq!(
            window.sidebar().selected(),
            Some(archive),
            "`g i` typed into the composer navigated away from the draft"
        );
    })
}
