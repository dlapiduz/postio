//! Marking several thread rows and pressing `a` archives the conversations.
//! #468.
//!
//! `thread_keystroke.rs` proves the *cursor* row case that #307 built: `a`
//! over one thread row archives that whole conversation. This is the half ADR
//! 0015 Q3 also asks for and #307 left:
//!
//! > **Selection stays a predicate.** Selecting thread rows selects threads
//! > […] the *store* expands threads to member messages inside the verbs.
//!
//! A thread row's `id` is its newest message in the folder — deliberately,
//! because that is what makes the row openable, draggable and repliable — so
//! a *selection* of thread rows is a selection of representatives, and the
//! verbs took it at its word. Six conversations marked, six messages
//! archived, and the rest of every one of them still sitting in the folder.
//!
//! That is the failure mode worth a test of its own: it is silent, it looks
//! like it worked, and the mail it leaves behind is mail the user believes
//! they have dealt with.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Nothing here dials anything — `start_syncing` is never called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::MailboxRole;
use postio_model::ids::{MessageId, ThreadId};
use postio_session::{Wiring, actions};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, Store, test_support};

/// Every message of `thread`, and which mailbox each is in right now.
async fn thread_mailboxes(database: &Store, thread: ThreadId) -> Vec<(MessageId, i64)> {
    let connection = database.connect().await.expect("a connection");
    let mut statement = connection
        .prepare("SELECT id, mailbox_id FROM messages WHERE thread_id = ?1 ORDER BY id")
        .await
        .expect("prepare");

    postio_storage::sql::mapped(&mut statement, [thread.get()], |row| {
        Ok((
            MessageId::new(postio_storage::sql::RowExt::col(row, 0)?),
            postio_storage::sql::RowExt::col::<i64>(row, 1)?,
        ))
    })
    .await
    .expect("query")
}

pub fn marking_two_thread_rows_archives_both_conversations() {
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
        let archive = report
            .mailbox(MailboxRole::Archive)
            .expect("the fixture has an archive folder")
            .id
            .get();
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

        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() > 0).await,
            "no rows to press a key on"
        );

        // ── two conversations, each of more than one message ─────────────────
        //
        // More than one is the whole point: on a single-message thread,
        // "archive the conversation" and "archive the representative" are the
        // same act and the bug is invisible.
        let model = list.model();
        let mut chosen: Vec<postio_gtk::list::Row> = Vec::new();
        for index in 0..model.n_items() {
            let Some(object) = model.item(index) else {
                continue;
            };
            let Ok(item) = object.downcast::<postio_gtk::list::MessageRow>() else {
                continue;
            };
            let Some(row) = item.row() else { continue };
            if !row.is_thread() || row.thread_count <= 1 {
                continue;
            }
            let matched = match row.thread {
                Some(thread) => thread_mailboxes(&database, thread)
                    .await
                    .iter()
                    .any(|(_, mailbox)| *mailbox != archive),
                None => false,
            };
            if matched {
                chosen.push(row);
            }
            if chosen.len() == 2 {
                break;
            }
        }
        assert_eq!(
            chosen.len(),
            2,
            "the folder does not hold two unarchived conversations of more than \
             one message, so this test cannot tell a conversation from its \
             newest message"
        );
        let threads: Vec<ThreadId> = chosen
            .iter()
            .map(|row| row.thread.expect("a thread row carries its thread"))
            .collect();
        let mut before: usize = 0;
        for thread in &threads {
            before += thread_mailboxes(&database, *thread).await.len();
        }
        assert!(
            before > 2,
            "two conversations of one message each is the case this cannot see"
        );

        // ── mark both rows, the way `x` does ────────────────────────────────
        for row in &chosen {
            list.select_message(row.id);
            while glib::MainContext::default().iteration(false) {}
            window.handle_key(
                gdk::Key::from_name("x").unwrap(),
                gdk::ModifierType::empty(),
            );
            while glib::MainContext::default().iteration(false) {}
        }

        // ── and archive ─────────────────────────────────────────────────────
        window.handle_key(
            gdk::Key::from_name("a").unwrap(),
            gdk::ModifierType::empty(),
        );

        let filed = settle_until(async || {
            for thread in &threads {
                let all_archived = thread_mailboxes(&database, *thread)
                    .await
                    .iter()
                    .all(|(_, mailbox)| *mailbox == archive);
                if !all_archived {
                    return false;
                }
            }
            true
        })
        .await;

        let mut archived: usize = 0;
        for thread in &threads {
            archived += thread_mailboxes(&database, *thread)
                .await
                .iter()
                .filter(|(_, mailbox)| *mailbox == archive)
                .count();
        }
        assert!(
            filed,
            "marking two thread rows and pressing `a` archived {archived} of the \
             {before} messages those two conversations hold. A thread row's id is \
             its newest message, so a selection of thread rows was taken as a \
             selection of representatives — which leaves the rest of every \
             conversation in the folder, silently, after a gesture that looked \
             like it worked (#468)."
        );

        bridge.shutdown();
    });
}
