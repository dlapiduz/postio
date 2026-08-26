//! Issue #436: the thread column's cursor and the reading pane.
//!
//! `t` drills into a conversation and `j`/`k` walk it, exactly as they walk
//! the mailbox. In the mailbox the pane follows the cursor — that is #325's
//! settled design, and `cursor_preview.rs` pins it. In the thread column it
//! did not follow anything: `ThreadView::connect_activated` fires on every
//! cursor move (`move_cursor` → `select_index` → `announce`), and **nothing
//! in the workspace was connected to it**. The signal was emitted into the
//! void, so walking a conversation left the pane on whatever the list had
//! last put there.
//!
//! That is the same class of fault as #325 and #70 Cause B — a surface that
//! is built, tested underneath, and joined to nothing — which is why the
//! assertion here is at the application level. `thread.rs`'s own tests prove
//! the column moves its cursor; only a test over the wired application can
//! fail when nobody listens.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store and `start_syncing` is never
//! called, so the thread is assembled from what `seed_small` filed — the
//! corpus' `list-thread-*` fixtures, threaded as sync would thread them.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
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

fn press(window: &Window, key: gdk::Key) {
    window.handle_key(key, gdk::ModifierType::empty());
}

/// Which mailbox holds `message`, straight out of the database.
fn mailbox_of(database: &Database, message: postio_model::ids::MessageId) -> i64 {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .expect("reading a seeded message must not fail")
        .expect("the message the column is showing must exist")
        .mailbox_id
        .get()
}

pub fn the_pane_follows_the_thread_columns_cursor() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-thread-cursor-{}", std::process::id()));
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
    seed_small(&database, 11);
    let store = database.clone();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let showing = wired.showing.clone();

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the list is empty, so there is no thread to drill into"
    );

    // ── find the longest conversation in the folder ─────────────────────
    // `seed_small` files the corpus' `list-thread-*` fixtures into weighted
    // random mailboxes, so which thread lands where is a property of the
    // seed. Take the *biggest* thread rather than the first one with more
    // than one message: the corpus contains short accidental threads as well
    // as the seven-message `list-thread-*` conversation, and only the long
    // one is spread across folders. Picking the first match found a
    // three-message thread sitting entirely in one folder, which cannot say
    // anything about #44.
    let model = list.model();
    let mut threaded: Option<postio_gtk::list::Row> = None;
    for index in 0..model.n_items() {
        let Some(object) = model.item(index) else {
            continue;
        };
        let Ok(item) = object.downcast::<postio_gtk::list::MessageRow>() else {
            continue;
        };
        let Some(row) = item.row() else { continue };
        if row.thread.is_none() || row.thread_count <= 1 {
            continue;
        }
        if threaded
            .as_ref()
            .is_none_or(|best| row.thread_count > best.thread_count)
        {
            threaded = Some(row);
        }
    }
    let threaded = threaded.expect(
        "no row in this folder belongs to a thread with more than one message, \
         so this seed cannot exercise the thread column",
    );

    // ── open it ─────────────────────────────────────────────────────────
    list.select_message(threaded.id);
    press(&window, gdk::Key::t);
    assert!(
        settle_until(|| window.thread_open()),
        "`t` did not open the thread column"
    );
    let thread = window.thread();
    // `open_thread` puts up what the list already had, synchronously, and
    // then reads the rest of the conversation. Wait for the *read*, not just
    // for the column: the first paint is the folder-scoped subset, and
    // asserting on it would be asserting on the bug #44 fixed.
    assert!(
        settle_until(|| thread.rows().len() as u32 >= threaded.thread_count),
        "the thread column filled with {} of the thread's {} messages",
        thread.rows().len(),
        threaded.thread_count
    );

    // ── #44: the column holds the whole conversation, not this folder's ─
    // `open_thread` carried a comment describing the *pre-#44* behaviour --
    // "the part of it in this folder is all the list model has ever been
    // able to offer" -- sitting directly beside code that no longer does
    // that. `ListScope::Thread` filters on `messages.thread_id` alone, with
    // no mailbox or account restriction, and `seed_small` files the corpus'
    // thread across weighted random mailboxes. So the column should be
    // showing messages from more than one folder, and if it is, the comment
    // was stale rather than the fix incomplete.
    let folders: std::collections::BTreeSet<i64> = thread
        .rows()
        .iter()
        .map(|row| mailbox_of(&store, row.id))
        .collect();
    assert!(
        folders.len() > 1,
        "the thread column is showing {} message(s) from a single folder \
         ({folders:?}), so this seed cannot tell a cross-folder thread from \
         a folder-scoped one",
        thread.rows().len()
    );

    // ── walk it ─────────────────────────────────────────────────────────
    // `k`, not `j`: `ThreadView::open` ends on `last_row`, because the newest
    // message is what a drill-in is usually about and it is where the canvas
    // draws the selection. The cursor therefore starts at the end of the
    // column and `j` has nowhere to go.
    let before = thread.cursor().expect("the thread column has a cursor");
    press(&window, gdk::Key::k);
    assert!(
        settle_until(|| thread.cursor() != Some(before)),
        "`k` did not move the thread column's cursor, so this test cannot \
         say anything about what the pane did"
    );
    let after = thread.cursor().expect("the thread column has a cursor");

    // ── and the pane follows, with no Return ────────────────────────────
    // The regression: `ThreadView::connect_activated` announced this move
    // and nothing was listening, so the pane went on showing whatever the
    // mailbox list had put there.
    assert!(
        settle_until(|| showing.get() == Some(after)),
        "the thread column's cursor moved to {after:?} and the reading pane \
         is showing {:?}. Nothing feeds the reader from the thread cursor.",
        showing.get()
    );
    assert!(
        window.reading(),
        "the pane claims to be showing the thread's message but is not reading"
    );
}
