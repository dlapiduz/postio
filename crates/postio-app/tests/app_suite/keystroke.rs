//! A keystroke, all the way to SQLite.
//!
//! `postio-bl2`, instance 1: every command in the application resolved
//! correctly through the registry, the keymap, the palette and the selection
//! model, and then reached `handler_fn(|_, _| async {})` — a literal no-op.
//! Archive, delete, flag and move were all inert, and nothing errored, so it
//! read as the application ignoring you.
//!
//! Every layer was tested. The keymap had tests, the registry had tests, the
//! dispatcher had tests, the verbs had tests over a real schema. What none of
//! them could see is that the bus had nobody on the other end.
//!
//! So this asserts the one thing none of them assert: press `a`, and the row
//! moves in the database. The assertion is a `SELECT`, deliberately — the far
//! end of the chain, as far from the keypress as it is possible to get while
//! staying in one process.
//!
//! Nothing here touches the network: `start_syncing` is never called, so no
//! socket is opened and the queued operation simply waits, which is exactly
//! what local-first means.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

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
use postio_model::ids::MessageId;
use postio_session::{Wiring, actions};
use postio_storage::repository::MessageRepository;
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

/// Which mailbox holds `message`, straight out of the database.
fn mailbox_of(database: &Database, message: MessageId) -> i64 {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .expect("a read")
        .expect("the message is still there")
        .mailbox_id
        .get()
}

#[test]
fn pressing_a_archives_the_row_in_the_database() {
    let state_dir = std::env::temp_dir().join(format!("postio-keystroke-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

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
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    // The real bus, over the real store — the piece that was a no-op.
    let state = SharedState::default();
    // Composed exactly as `run` composes it, through the same `wire`.
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    assert!(
        wired.contains(&CommandId::Archive),
        "the bus does not answer archive, so this test cannot mean anything"
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

    // `j` puts the cursor on a row. Nothing is *selected* — a plain click
    // clears the selection in this list — so this is also the daily case:
    // an action with an empty selection has to act on the cursor row.
    window.handle_key(
        gdk::Key::from_name("j").unwrap(),
        gdk::ModifierType::empty(),
    );
    while glib::MainContext::default().iteration(false) {}
    let focused = list.cursor_id().expect("`j` moved the cursor onto a row");
    let before = mailbox_of(&database, focused);
    assert_ne!(
        before, archive,
        "the fixture put the first row in the archive already, so archiving it \
         would prove nothing"
    );

    // ── the keystroke ───────────────────────────────────────────────────
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );

    // The bus runs on the runtime's threads, so the write lands a moment
    // after the key press. Poll the database rather than the widget: the
    // widget is what every other test already checks.
    let moved = settle_until(|| mailbox_of(&database, focused) == archive);

    assert!(
        moved,
        "`a` resolved through the keymap and the registry and the row never \
         moved. Every layer in that chain has its own passing tests; what has \
         no test without this one is whether they are joined. See postio-bl2."
    );

    bridge.shutdown();
}
