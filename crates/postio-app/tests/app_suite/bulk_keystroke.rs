//! `Ctrl+A` then `U`, all the way to SQLite.
//!
//! The sibling of `keystroke.rs`, for the gesture that is not a selection.
//! Marking a whole mailbox read is the second thing anyone does to an
//! 81,717-message account, and it travels a different road from `a` on a
//! focused row: the window's select-all, `commands::mirror` keeping it a
//! predicate rather than a list of ids, `AppState::resolve` handing back
//! `Resolved::Everything`, and a bulk statement in `actions`. Every joint on
//! that road has its own tests. What none of them can see is whether the road
//! joins up — which is exactly the failure `keystroke.rs` was written for.
//!
//! The assertion is a `SELECT` over every row the folder holds, deliberately:
//! the far end of the chain, as far from the key press as one process reaches.
//!
//! Nothing here touches the network. `start_syncing` is never called, so the
//! queue rows the write leaves simply wait.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{MailboxId, MailboxRole};
use postio_session::{Wiring, actions};
use postio_storage::repository::{ColumnFlag, MessageRepository, MessageSet};
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

/// How many messages in `mailbox` are still unread, straight out of the
/// database. A count rather than a read, for the same reason the verb uses one.
fn unread_in(database: &Database, mailbox: MailboxId) -> u32 {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .count_set(&MessageSet::in_mailbox(mailbox).with_flag(ColumnFlag::Seen, false))
        .expect("a count")
}

pub fn ctrl_a_then_shift_u_marks_the_whole_folder_read() {
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

    let database = test_support::memory();
    let report = seed_small(&database, 17);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    assert!(
        wired.contains(&CommandId::MarkUnread),
        "the bus does not answer mark-unread, so this test cannot mean anything"
    );

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
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the fixture has an inbox");
    // The header title-cases the server's `INBOX`, so this is the same folder
    // by the only name both halves agree on.
    assert!(
        list.mailbox_name().eq_ignore_ascii_case(&inbox.name),
        "the list opened on `{}` rather than the inbox, so `Ctrl+A` would \
         select some other folder",
        list.mailbox_name()
    );
    let mailbox = inbox.id;
    let before = unread_in(&database, mailbox);
    assert!(
        before > 1,
        "the fixture left {before} unread messages in the open folder, so \
         marking them all read would prove nothing about doing it in bulk"
    );

    // ── the gesture ─────────────────────────────────────────────────────
    // `Ctrl+A` never leaves the window: it is the list's own selection model
    // moving. What crosses to the engine is what `commands::mirror` makes of
    // it at send time, which has to still be a predicate.
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    while glib::MainContext::default().iteration(false) {}

    // `U`, not `u` — `u` is undo (docs/PRODUCT.md §16). The keymap folds the
    // shift into the character, so this is the chord the registry spells "U".
    window.handle_key(
        gdk::Key::from_name("U").unwrap(),
        gdk::ModifierType::SHIFT_MASK,
    );

    let read = settle_until(|| unread_in(&database, mailbox) == 0);

    assert!(
        read,
        "`Ctrl+A` then `U` resolved through the keymap, the registry, the \
         selection model and the mirror, and {} of {before} messages are still \
         unread. Until postio-t3u9 this path answered `Rejected` instead of \
         acting; a test that only asks whether the verb works cannot tell the \
         two apart.",
        unread_in(&database, mailbox)
    );

    bridge.shutdown();
}
