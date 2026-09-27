//! Hostile mail opened the way a person opens it (spec 006 US3, T104).
//!
//! `postio-render`'s hostile suite proves each fixture is contained and
//! bounded against the renderer alone. This is the application half: each
//! one stored, listed and opened from the message list, and after each the
//! pane shows a drawn body or the plain-text fallback -- never a blank pane
//! -- and the window still answers the next keystroke. The process being
//! alive to assert anything is the third claim.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::test_corpus::{self, Category};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, test_support};

pub fn each_hostile_message_opens_and_the_app_keeps_answering() {
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
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");

        let fixtures = test_corpus::by_category(Category::Hostile);
        assert!(fixtures.len() >= 5, "the corpus has its hostile fixtures");
        let mut stored = Vec::new();
        let account = {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let repository = MessageRepository::new(&connection);
            // Newest first in the list: an unflagged one on top, for
            // `cursor_preview`'s reason, then the fixtures.
            for (index, fixture) in std::iter::once(None)
                .chain(fixtures.iter().map(Some))
                .enumerate()
            {
                let received = chrono::Utc::now() - chrono::Duration::minutes(index as i64);
                let mut message = Message::new(account.id, inbox, received);
                message.subject = Some(fixture.map_or("Unflagged", |f| f.name()).to_owned());
                message.from = vec![EmailAddress::new(None::<&str>, "sender@example.com")];
                message.sync.body_state = BodyState::Full;
                let id = repository.create(&mut message).await.expect("a message");
                let parsed = fixture.map(|f| postio_model::mime::parse(f.bytes()));
                let body = StoredBody {
                    text: parsed
                        .as_ref()
                        .map_or(Some("x".to_owned()), |p| p.body.text.clone()),
                    html: parsed.as_ref().and_then(|p| p.body.html.clone()),
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                };
                repository
                    .set_body(id, &body, BodyState::Full)
                    .await
                    .expect("a body");
                if let Some(fixture) = fixture {
                    stored.push((id, fixture.name()));
                }
            }
            connection
                .execute(
                    "UPDATE messages SET flagged = 1 WHERE id NOT IN \
                     (SELECT id FROM messages ORDER BY received_at DESC LIMIT 1)",
                    (),
                )
                .await
                .expect("the fixture writes");
            account.id
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
        while glib::MainContext::default().iteration(false) {}
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the store has an account");
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 0).await,
            "the opening folder never filled"
        );
        wired
            .feeds
            .messages
            .open(postio_model::ListScope::Flagged(account));
        assert!(
            settle_until(async || list.model().n_items() as usize == stored.len()).await,
            "the Flagged view never filled"
        );

        for (message, name) in &stored {
            let before = window.reader().view().document().map(|d| d.generation);
            list.select_message(*message);
            assert!(
                settle_until(async || {
                    window
                        .reader()
                        .view()
                        .document()
                        .is_some_and(|d| Some(d.generation) != before)
                })
                .await,
                "{name}: the pane never drew the message or its fallback"
            );
            let document = window.reader().view().document().expect("a snapshot");
            assert!(
                matches!(document.outcome, postio_render::Outcome::Rendered)
                    || !document.text.text.trim().is_empty(),
                "{name}: the pane is blank -- neither a drawn body nor the \
                 fallback's text"
            );
            // And the window still answers: the next keystroke moves the
            // cursor.
            let cursor = list.cursor_id();
            assert_eq!(
                window.handle_key(gdk::Key::j, gdk::ModifierType::empty()),
                glib::Propagation::Stop,
                "{name}: `j` was not answered after opening it"
            );
            if Some(*message) != stored.last().map(|(id, _)| *id) {
                assert!(
                    settle_until(async || list.cursor_id() != cursor).await,
                    "{name}: the window stopped responding after opening it"
                );
            }
        }

        bridge.shutdown();
    });
}
