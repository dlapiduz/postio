//! A body arriving fills in the open conversation, not just the single pane.
//!
//! This is the inbox bug a real store surfaced (`feature/turso-store`): a
//! folder threads, so its messages open in the one-document conversation pane
//! (ADR 0032), and on a store whose backfill is far behind — which a first
//! sync of tens of thousands of messages is — the messages open
//! `headers_only` with no body. `fill_thread` requests each missing body, but
//! nothing drew it when it landed: `body_arrived` repainted only the single
//! reader, and the conversation half had been removed as "coalesced through
//! `set_thread_body`" without anything calling `set_thread_body` on an
//! arrival. So a conversation whose bodies were not yet local stayed a stack
//! of empty headers until it was closed and reopened.
//!
//! The seam is `body_arrives.rs`': write the body, hand `Feeds::apply` the
//! `BodyLoaded` a real engine would emit, and assert on the document that
//! reached WebKit.

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
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::{BlobStore, test_support};

const BODY: &str = "the figures you asked for are attached";

pub fn a_body_arriving_fills_in_the_open_conversation() {
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

        // One message, in a thread, with headers only — its body is not local,
        // exactly as a first sync leaves it before the backfill arrives.
        let (_thread, message) = {
            let connection = database.connect().await.expect("a connection");
            let mut thread = postio_model::Thread::new(account.id);
            let thread = ThreadRepository::new(&connection)
                .create(&mut thread)
                .await
                .expect("create the thread");
            let mut msg = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
            msg.subject = Some("Quarterly figures".to_owned());
            msg.from = vec![postio_model::EmailAddress::new(
                Some("Ada Lovelace"),
                "ada@example.com",
            )];
            msg.sync.body_state = postio_model::BodyState::HeadersOnly;
            let id = MessageRepository::new(&connection)
                .create(&mut msg)
                .await
                .expect("create the message");
            ThreadRepository::new(&connection)
                .add_message(thread, id)
                .await
                .expect("join the message to the thread");
            (thread, id)
        };

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

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 0).await,
            "the seeded thread never reached the list"
        );
        list.first_row();
        let cursor = list.cursor_row().expect("a row to land on");
        window.open_conversation(&cursor);
        assert!(
            settle_until(async || window.conversation().len() == 1).await,
            "opening the thread never filled the pane"
        );

        // The control: the conversation drew the message, and its body is not
        // in the document, because it is not local yet.
        assert!(
            settle_until(async || window
                .conversation()
                .thread_document()
                .is_some_and(|d| d.matches("<details").count() == 1))
            .await,
            "the conversation never drew the message"
        );
        assert!(
            !window
                .conversation()
                .thread_document()
                .unwrap_or_default()
                .contains(BODY),
            "the body is not local yet, so it cannot be in the document"
        );

        // The body lands — write it, then hand the pane the event a real
        // engine emits on commit.
        MessageRepository::new(&database.connect().await.expect("a connection"))
            .set_body(
                message,
                &StoredBody {
                    text: Some(BODY.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("store the body");
        wired.feeds.apply(&Event::BodyLoaded {
            account: account.id,
            message,
        });

        assert!(
            settle_until(async || window
                .conversation()
                .thread_document()
                .is_some_and(|d| d.contains(BODY)))
            .await,
            "the body landed for the message the conversation is showing and the \
             pane went on showing an empty header. `Event::BodyLoaded` has to \
             reach the one-document pane, not only the single reader"
        );

        window.destroy();
    });
}
