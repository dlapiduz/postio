#![allow(unsafe_code)]
//! The next conversation is read and rendered before `j` reaches it.
//!
//! Landing on a conversation used to begin its work at the keystroke: read
//! each body from the store, then on the main thread judge each for reader
//! view and sanitise it -- two html5ever parses per message, in front of
//! the first frame of the conversation. While the reader looks at one
//! conversation, the next one is prepared on the runtime, so the keystroke
//! costs the main thread neither parse.
//!
//! Counted at the parsers, on the main thread: the preparation's own parses
//! happen on a worker, and the thread-local counters do not see them, which
//! is exactly the distinction this is about.

use crate::settle_until;

use gtk::gdk;
use gtk::prelude::*;
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{BodyState, EmailAddress, Message, Thread};
use postio_session::Wiring;
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::test_support;

/// Two conversations of three, the older one's bodies marked so the test
/// can tell when it is the one drawn.
async fn two_conversations(database: &postio_storage::Store) {
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    for (conversation, minutes) in [("newer", 60), ("older", 0)] {
        let mut thread = Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .await
            .expect("a thread");
        for n in 0..3 {
            let mut message = Message::new(
                account.id,
                inbox,
                chrono::Utc::now() - chrono::Duration::minutes(120 - minutes - n),
            );
            message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
            message.subject = Some(format!("{conversation} interlock report"));
            let id = MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            ThreadRepository::new(&connection)
                .add_message(thread.id, id)
                .await
                .expect("joined");
            MessageRepository::new(&connection)
                .set_body(
                    id,
                    &StoredBody {
                        html: Some(format!(
                            "<table><tr><td>The {conversation} readings, part {n}.</td></tr></table>"
                        )),
                        ..StoredBody::default()
                    },
                    BodyState::Full,
                )
                .await
                .expect("a body");
        }
    }
}

pub fn the_next_conversation_is_drawn_without_parsing_on_the_main_thread() {
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
        two_conversations(&database).await;
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

        let window = Window::default();
        window.present();
        let _ = feed_the_window(&window, &wiring).await;
        let drawn = |text: &'static str| {
            let window = window.clone();
            move || {
                window
                    .conversation()
                    .thread_document()
                    .is_some_and(|document| document.contains(text))
            }
        };
        let newer = drawn("The newer readings, part 2.");
        assert!(
            settle_until(async || newer()).await,
            "the first conversation never drew its bodies"
        );
        // A moment for the neighbour to be prepared, which is the reading
        // time a person spends on the first one.
        for _ in 0..40 {
            while gtk::glib::MainContext::default().iteration(false) {}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let judged = postio_ui::test_support::bulk_judged();
        let sanitised = postio_ui::test_support::bodies_sanitised();
        window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
        let older = drawn("The older readings, part 2.");
        assert!(
            settle_until(async || older()).await,
            "`j` never drew the next conversation's bodies"
        );
        let judged = postio_ui::test_support::bulk_judged() - judged;
        let sanitised = postio_ui::test_support::bodies_sanitised() - sanitised;
        eprintln!("DIAG after j: judged={judged} sanitised={sanitised}");
        assert_eq!(
            (judged, sanitised),
            (0, 0),
            "`j` into a conversation prepared ahead still parsed its bodies on \
             the main thread"
        );
        window.close();
    });
}
