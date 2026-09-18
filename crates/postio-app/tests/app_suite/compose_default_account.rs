//! A new message comes from the account marked default, and only a new one.
//!
//! #960 put the marker in the store -- a column, a command, a badge saying
//! "new messages come from this account" -- and, deliberately, nothing read
//! it. This is the reader (#1161), with #960's fence around it: compose with
//! no originating message reads the marker, a `mailto:` link reads it, and a
//! reply does not, because which address a reply comes from is decided by
//! the message being answered, and a default that overrode that would send
//! replies from the wrong address to people who wrote to a different one.
//!
//! Nothing here touches the network: `feed_the_window` reads the local
//! store and `start_syncing` is never called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::mailto::Mailto;
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::seed::{seed_extra_account, seed_small};
use postio_storage::{BlobStore, test_support};

pub fn a_new_message_comes_from_the_default_account_and_a_reply_does_not() {
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
        let first = report.account.id;
        // A second account, marked default. The first stays the one the
        // window opens on -- sidebar order is not the marker's business.
        let second = seed_extra_account(&database, "Home", "home@example.net", 12)
            .await
            .account
            .id;
        {
            let connection = database.connect().await.expect("a connection");
            AccountRepository::new(&connection)
                .set_default(second)
                .await
                .expect("mark the second account default");
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
        while glib::MainContext::default().iteration(false) {}
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account");

        // The fence first, on the row the window opened on: a reply comes from
        // the account the message arrived in, whatever is marked default.
        // Activated through `GtkListView`'s own `list.activate-item`, the way
        // `reply_source.rs` does it, once the folder has filled.
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 0).await,
            "the opening folder never filled"
        );
        list.first_row();
        list.test_activate_cursor();
        // The first row of the fixture is a thread, so what opens is the
        // conversation pane rather than the single-message reader; `e`
        // replies from either, and the wait has to accept either.
        assert!(
            settle_until(async || window.reading() || window.conversation().is_mapped()).await,
            "the window never showed its first message"
        );
        window.handle_key(gdk::Key::e, gdk::ModifierType::empty());
        let opened = settle_until(async || window.composer().is_open()).await;
        assert!(opened, "`e` did not open the composer");
        assert_eq!(
            window.composer().draft().account_id,
            first,
            "a reply comes from the account the message arrived in, whatever is \
             marked default -- #960's fence"
        );
        window.composer().discard();
        let closed = settle_until(async || !window.composer().is_open()).await;
        assert!(closed, "discarding the reply left the composer open");

        // `c`: a message with no origin comes from the marked account.
        window.list().grab_focus();
        while glib::MainContext::default().iteration(false) {}
        window.handle_key(gdk::Key::c, gdk::ModifierType::empty());
        let opened = settle_until(async || window.composer().is_open()).await;
        assert!(opened, "`c` did not open the composer");
        assert_eq!(
            window.composer().draft().account_id,
            second,
            "a new message should come from the account marked default, not the \
             first one in the table"
        );
        window.composer().discard();
        let closed = settle_until(async || !window.composer().is_open()).await;
        assert!(closed, "discarding the new message left the composer open");

        // A mailto: link is a new message too.
        window.deliver_mailto(Mailto::parse("mailto:ada@example.com").expect("a mailto uri"));
        let opened = settle_until(async || window.composer().is_open()).await;
        assert!(opened, "the mailto link did not open the composer");
        assert_eq!(
            window.composer().draft().account_id,
            second,
            "a mailto link is a new message and comes from the default account"
        );

        let _ = wired;
        bridge.shutdown();
    });
}
