//! The folder header's "N unread" follows a reload, not only a folder change.
//!
//! A real store surfaced it (`feature/turso-store`): the header above the
//! rows read "32 unread" while the sidebar badge and the store both said 2.
//! `set_mailbox` — which draws that count — was called only when a folder was
//! *opened*, so a resync that moved the real count updated the sidebar (it
//! re-reads on every load) and left the header frozen at its open-time value.
//! `folders.connect_loaded` refreshes it now, on every load, for the folder
//! on screen.
//!
//! Nothing here dials: the count is changed in the store and the
//! `MailboxesChanged` a resync would emit is handed to `Feeds::apply`, the
//! same seam `body_arrives.rs` uses.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts.

use crate::{settle, settle_until};
use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::Event;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, Store, test_support};

async fn unread_message(
    database: &Store,
    account: postio_model::ids::AccountId,
    inbox: postio_model::ids::MailboxId,
    subject: &str,
) {
    let connection = database.connect().await.expect("a connection");
    let mut msg = postio_model::Message::new(account, inbox, chrono::Utc::now());
    msg.subject = Some(subject.to_owned());
    msg.from = vec![postio_model::EmailAddress::new(
        Some("Ada"),
        "ada@example.com",
    )];
    // No `Flag::Seen`: an unread arrival, which the unread-count trigger bumps.
    MessageRepository::new(&connection)
        .create(&mut msg)
        .await
        .expect("create the message");
}

pub fn the_header_count_follows_a_reload() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test, before the app runs.
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
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            test_support::account_with_inbox(&connection).await
        };
        unread_message(&database, account.id, inbox, "one").await;
        unread_message(&database, account.id, inbox, "two").await;

        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
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
        settle();
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        assert!(
            settle_until(async || window.list().model().n_items() > 0).await,
            "the inbox never reached the list"
        );

        window.open_mailbox(inbox);
        assert!(
            settle_until(async || window.list().header_unread() == 2).await,
            "opening the inbox did not put its unread count in the header"
        );

        // A third unread message arrives, as a resync would add it, and the
        // event a resync emits is handed to the feed.
        unread_message(&database, account.id, inbox, "three").await;
        wired.feeds.apply(&Event::MailboxesChanged {
            account: account.id,
        });

        assert!(
            settle_until(async || window.list().header_unread() == 3).await,
            "the folder's unread count changed and the header above the rows \
             kept its open-time value — the sidebar refreshes on a reload and \
             the header has to as well, or the two disagree (a real store read \
             '32 unread' over two)"
        );

        window.destroy();
    });
}
