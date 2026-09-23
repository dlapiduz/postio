#![allow(unsafe_code)]
//! Opening a conversation reads its messages in one crossing to the runtime
//! (#1609).
//!
//! `fill_thread` issued one `read_then` per message: a thirty-message thread
//! was thirty runtime tasks, thirty store turns and thirty channel wake-ups on
//! the main context before a byte of body was drawn, on every cursor move
//! onto it. Counted at the crossing, not timed.

use crate::settle_until;

use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{EmailAddress, Message, Thread};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::test_support;

const MEMBERS: usize = 12;

pub fn a_conversation_is_read_in_one_crossing() {
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
        {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let mut thread = Thread::new(account.id);
            ThreadRepository::new(&connection)
                .create(&mut thread)
                .await
                .expect("a thread");
            for n in 0..MEMBERS {
                let mut message = Message::new(
                    account.id,
                    inbox,
                    chrono::Utc::now() + chrono::Duration::minutes(n as i64),
                );
                message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
                message.subject = Some(format!("interlock, message {n}"));
                let id = MessageRepository::new(&connection)
                    .create(&mut message)
                    .await
                    .expect("a message");
                ThreadRepository::new(&connection)
                    .add_message(thread.id, id)
                    .await
                    .expect("joined");
            }
        }
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = postio_storage::BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            postio_core::bridge::event_channel().0,
            bridge.commands(),
        );

        let before = postio_app::reading::thread_crossings();
        let window = Window::default();
        window.present();
        let _ = feed_the_window(&window, &wiring).await;
        assert!(
            settle_until(async || window.conversation().len() == MEMBERS).await,
            "the autoselected row never opened its conversation"
        );
        let crossings = postio_app::reading::thread_crossings() - before;
        // Two, not one: the pane reports a conversation opened twice while it
        // opens -- once with the row the list handed it, once with the whole
        // column -- and each is a fill. What must not happen is a crossing
        // per message: before #1609 this was 13 for 12 messages.
        assert!(
            crossings <= 2,
            "opening a {MEMBERS}-message conversation crossed to the runtime \
             {crossings} times to read its messages"
        );
        window.close();
    });
}
