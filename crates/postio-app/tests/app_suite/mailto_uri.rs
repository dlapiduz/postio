//! A `mailto:` link, all the way to a composer with the address in it.
//!
//! The desktop entry has said `MimeType=x-scheme-handler/mailto` and
//! `Exec=postio %U` since the first release, so a browser hands Postio the
//! link a person clicked — and until now the application was built without
//! `HANDLES_OPEN`, so GTK dropped the URI on the floor and launched an empty
//! window. Every layer looked right: the entry registered, the composer
//! opened, the draft model had the fields. Nothing joined them.
//!
//! So this asserts the join, in both orders it can happen: a link arriving
//! at a window whose store is already fed, and one arriving first, before
//! there is an account to compose from — a cold launch from a browser is
//! exactly that — which has to wait and then open, not be lost.
//!
//! Nothing here touches the network: `feed_the_window` reads the local
//! store and `start_syncing` is never called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::mailto::Mailto;
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn a_mailto_link_opens_the_composer_with_the_address_filled_in() {
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
        let account = report.account.id;
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

        // The cold launch: the link is there before the store is. It must
        // wait for an account, not open a composer nobody can send from and
        // not be forgotten.
        let early = Mailto::parse("mailto:ada@example.com?subject=Lunch%20on%20Thursday")
            .expect("a mailto uri");
        window.deliver_mailto(early);
        while glib::MainContext::default().iteration(false) {}
        assert!(
            !window.composer().is_open(),
            "a mailto arriving before the store opened a composer with no account behind it"
        );

        // The same call `run` makes.
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account");
        let opened = settle_until(async || window.composer().is_open()).await;
        assert!(
            opened,
            "the mailto that arrived before the store was fed never opened the composer \
             once there was an account to compose from"
        );
        let draft = window.composer().draft();
        assert_eq!(
            draft.account_id, account,
            "the draft is not for the account that was fed"
        );
        assert_eq!(
            draft
                .to
                .iter()
                .map(|a| a.address.as_str())
                .collect::<Vec<_>>(),
            ["ada@example.com"]
        );
        assert_eq!(draft.subject, "Lunch on Thursday");

        // The warm case: a second link while the composer is already open is
        // the one-composition-at-a-time rule, so close it first and hand
        // over a fresh one — the way a second click in a browser reaches a
        // running Postio.
        window.composer().close();
        while glib::MainContext::default().iteration(false) {}
        assert!(
            !window.composer().is_open(),
            "closing the composer left it open"
        );

        let later = Mailto::parse("mailto:grace@example.net?cc=alan@example.org&body=Noon%3F")
            .expect("a mailto uri");
        window.deliver_mailto(later);
        let opened = settle_until(async || window.composer().is_open()).await;
        assert!(
            opened,
            "a mailto delivered to a fed window did not open the composer"
        );
        let draft = window.composer().draft();
        assert_eq!(
            draft
                .to
                .iter()
                .map(|a| a.address.as_str())
                .collect::<Vec<_>>(),
            ["grace@example.net"]
        );
        assert_eq!(
            draft
                .cc
                .iter()
                .map(|a| a.address.as_str())
                .collect::<Vec<_>>(),
            ["alan@example.org"]
        );
        assert_eq!(draft.body.text.as_deref(), Some("Noon?"));

        let _ = wired;
        bridge.shutdown();
    });
}
