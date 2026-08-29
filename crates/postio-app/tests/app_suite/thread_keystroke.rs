//! `a` on a thread row archives the whole conversation (ADR 0015 Q3, #307).
//!
//! `keystroke.rs` proves the chain from a key to a row moving in the
//! database. This proves the thing ADR 0015 changed about what that key
//! *means*: in a folder the list shows one row per conversation, and acting
//! on "the row" has to act on the conversation rather than on the one message
//! the row happens to be drawn from.
//!
//! It is deliberately at this level. `postio-storage` knows the thread query,
//! `postio-runtime` knows the window, `postio-gtk` knows the row, and
//! `postio-app` is the only place that knows a key pressed over a thread row
//! should reach the store as a thread. Every one of those layers passes its
//! own tests with the join missing — which is exactly how #70 and #325
//! happened.
//!
//! # It does not dial anything
//!
//! The bus is composed over the local store through the same `actions::wire`
//! the application uses; `start_syncing` is never called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

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
use postio_storage::{BlobStore, Database, test_support};

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// Every message of `thread`, and which mailbox each is in right now.
fn thread_mailboxes(database: &Database, thread: ThreadId) -> Vec<(MessageId, i64)> {
    let connection = database.connection().expect("a connection");
    let mut statement = connection
        .prepare("SELECT id, mailbox_id FROM messages WHERE thread_id = ?1 ORDER BY id")
        .expect("prepare");
    let rows = statement
        .query_map([thread.get()], |row| {
            Ok((MessageId::new(row.get(0)?), row.get::<_, i64>(1)?))
        })
        .expect("read the conversation");
    rows.collect::<Result<Vec<_>, _>>().expect("collect")
}

pub fn pressing_a_on_a_thread_row_archives_the_whole_conversation() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let archive = report
        .mailbox(MailboxRole::Archive)
        .expect("the fixture has an archive folder")
        .id
        .get();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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
        .expect("the seeded store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "no rows to press a key on"
    );

    // ── find a row that is a conversation of more than one message ──────
    // The folder threads now, so every row here is a thread row; this picks
    // the one where "the conversation" and "the message it is drawn from"
    // are demonstrably different things.
    let model = list.model();
    let mut chosen: Option<postio_gtk::list::Row> = None;
    for index in 0..model.n_items() {
        let Some(object) = model.item(index) else {
            continue;
        };
        let Ok(item) = object.downcast::<postio_gtk::list::MessageRow>() else {
            continue;
        };
        let Some(row) = item.row() else { continue };
        if !row.is_thread() {
            continue;
        }
        if chosen
            .as_ref()
            .is_none_or(|best| row.thread_count > best.thread_count)
        {
            chosen = Some(row);
        }
    }
    let chosen = chosen.expect(
        "the folder shows no thread rows at all, so the list is not threaded \
         and this test cannot mean anything",
    );
    assert!(
        chosen.thread_count > 1,
        "the biggest conversation in this folder holds one message, so \
         archiving it would not tell a thread from a message"
    );
    let thread = chosen.thread.expect("a thread row carries its thread");

    let before = thread_mailboxes(&database, thread);
    assert!(
        before.len() > 1,
        "the store disagrees with the row about the size of the conversation"
    );
    assert!(
        before.iter().any(|(_, mailbox)| *mailbox != archive),
        "every message of this conversation is already archived, so archiving \
         it would prove nothing"
    );

    list.select_message(chosen.id);
    while glib::MainContext::default().iteration(false) {}

    // ── the keystroke ───────────────────────────────────────────────────
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );

    let filed = settle_until(|| {
        thread_mailboxes(&database, thread)
            .iter()
            .all(|(_, mailbox)| *mailbox == archive)
    });

    assert!(
        filed,
        "`a` on a thread row archived {:?} of the conversation's {} messages. \
         Acting on 'the row' and acting on 'one message of six' cannot both \
         be what the key means, and the row is a conversation.",
        thread_mailboxes(&database, thread)
            .iter()
            .filter(|(_, mailbox)| *mailbox == archive)
            .count(),
        before.len()
    );

    bridge.shutdown();
}
