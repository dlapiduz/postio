//! Recipient completion answers from memory, not from the store.
//!
//! The composer asks for candidates on every keystroke in `To`, on the GTK
//! thread. Each ask used to open a store connection and run a contacts query
//! whose leading-wildcard `LIKE`s scan the table -- a blocking read per key
//! on the thread that draws. Now the directory is read off the thread when
//! the composer opens, and a keystroke is arithmetic over what was read.
//!
//! So the budget is a count, in the style of the other gates here: typing a
//! name letter by letter opens no connections at all, and still offers the
//! contact.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::{settle, settle_until};
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;
use postio_session::Wiring;
use postio_storage::repository::ContactRepository;
use postio_storage::seed::seed_small;
use postio_storage::test_support::counting::checkouts;
use postio_storage::{BlobStore, test_support};

pub fn typing_a_recipient_opens_no_connections_and_still_completes() {
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
        {
            // A person the test names, rather than one the seed happens to
            // hold. People are shared across accounts, so completion in the
            // seed's account offers them.
            let connection = database.connect().await.expect("checkout");
            ContactRepository::new(&connection)
                .create(
                    Some("Wilhelmina Quartz"),
                    &[EmailAddress::new(None::<String>, "wilhelmina@example.com")],
                )
                .await
                .expect("create the contact");
        }
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
        let _wired = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account");
        settle();

        window.handle_key(gdk::Key::c, gdk::ModifierType::empty());
        let composer = window.composer();
        assert!(composer.is_open(), "`c` did not open the composer");

        // The directory is read when the composer opens; wait for it the way
        // a person would -- by typing and seeing the contact offered.
        // Alternating, because an unchanged text asks nothing, and the first
        // ask may land before the read does.
        let flip = std::cell::Cell::new(false);
        let offered = settle_until(async || {
            composer.test_set_to(if flip.replace(!flip.get()) {
                "wil"
            } else {
                "wilh"
            });
            composer.test_recipient_suggestion_count() > 0
        })
        .await;
        assert!(offered, "the composer never offered the contact");

        // ── now the budget: a name typed letter by letter ────────────────────
        let before = checkouts();
        for typed in ["w", "wi", "wil", "wilh", "wilhe", "quar", "quartz"] {
            composer.test_set_to(typed);
            while glib::MainContext::default().iteration(false) {}
            assert!(
                composer.test_recipient_suggestion_count() > 0,
                "`{typed}` offered nothing"
            );
        }
        assert_eq!(
            checkouts() - before,
            0,
            "typing a recipient opened store connections on the thread that draws"
        );

        bridge.shutdown();
    });
}
