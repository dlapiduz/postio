//! Issue #749: the reading pane flashes black between messages.
//!
//! Every render in this pane is a full document teardown and reload —
//! JavaScript is off, so there is no incremental path, and the page cache is
//! off too. Between the old document being discarded and the new one's first
//! paint the `WebView` has nothing of its own to draw, and under the GTK4 GL
//! path that reads as black. So the count that matters to a person is not how
//! often the pane was *asked* to draw; it is how often a document was
//! actually handed to WebKit, because that is how many chances there were to
//! see the gap.
//!
//! Three ways the application was spending loads it did not need:
//!
//! 1. the filler is wired to `connect_cursor_moved` **and**
//!    `connect_activated`, and only the cursor deduplicates — so `Enter` (or
//!    a double click) on the row already under the cursor rebuilt the same
//!    document a second time;
//! 2. `Fill::repaint` re-rendered unconditionally, and `Event::BodyLoaded` is
//!    emitted for every *payload* a backfill commits — so a message being
//!    read was torn down and rebuilt byte-for-byte identically, repeatedly;
//! 3. and each of those reloads reset the reader's scroll position, so a body
//!    someone was halfway down jumped back to the top for no visible reason.
//!
//! `paints()` cannot see any of this: it counts the asking. `loads()` counts
//! the cost, which is why the assertions here are on `loads()` and why the
//! coalescing assertion in `body_arrives.rs` stays on `paints()` — the two
//! measure deliberately different things.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket and this never calls it. An arrival is simulated the way
//! `body_arrives.rs` does it — the store is written, then `Feeds::apply` is
//! handed the event a real engine would emit.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::Event;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::MessageId;
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
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

/// Give the application every chance to load a document, and answer whether
/// it left `held` true throughout — the mirror of [`settle_until`], for the
/// criteria that are about something *not* happening.
fn settle_while(held: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if !held() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    held()
}

/// `j`, through the keymap the application actually runs.
fn press_j(window: &Window) {
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
}

pub fn moving_and_reopening_a_message_costs_one_document_load_each() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── two messages, both with bodies, so every pane state is a rendered
    //    document rather than a plate ──────────────────────────────────────
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (account, newest, older) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);

        let mut older = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::hours(1),
        );
        older.subject = Some("Last week's figures".into());
        older.sync.body_state = BodyState::Full;
        let older = repository.create(&mut older).expect("a message");

        let mut newest = Message::new(account.id, inbox, chrono::Utc::now());
        newest.subject = Some("Quarterly figures".into());
        newest.sync.body_state = BodyState::Full;
        let newest = repository.create(&mut newest).expect("a message");

        (account.id, newest, older)
    };
    // Different text in each, so the two documents genuinely differ and a
    // missing load would be a *visible* failure rather than a bookkeeping one.
    store_body(&database, newest, "See the attached figures.");
    store_body(&database, older, "Last week, for comparison.");

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ────────────────────────────────────────
    let wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 1),
        "need two rows to move between"
    );
    // The autoselect fills the pane for row 0 (#601) — that is the state the
    // rest of this starts from, not something under test here.
    assert!(
        settle_until(|| window.reading()),
        "the window opened with a row under the cursor and an empty pane"
    );
    assert!(
        settle_until(|| window.reader().loads() > 0),
        "the pane never loaded a document at all"
    );
    // Let the opening settle completely, so what follows is measured against
    // a pane that has stopped moving.
    let opened = {
        let mut steady = window.reader().loads();
        while !settle_while(|| window.reader().loads() == steady) {
            steady = window.reader().loads();
        }
        steady
    };

    // ── 1. moving the cursor to another message: exactly one load ────────
    press_j(&window);
    assert!(
        settle_until(|| window.reader().loads() > opened),
        "moving the cursor to the next message never reached the pane"
    );
    assert!(settle_while(|| window.reader().loads() == opened + 1));
    assert_eq!(
        window.reader().loads(),
        opened + 1,
        "moving the cursor one row should build one document, not {}",
        window.reader().loads() - opened
    );

    // ── 2. activating the row already under the cursor: no further load ──
    //
    // The filler is wired to the cursor *and* to activation, deliberately —
    // on a window nobody has touched the cursor has reported nothing, and
    // Enter still has to open what it is sitting on. What it must not do is
    // rebuild a document that is already on screen.
    let moved = window.reader().loads();
    activate_row(&window, 1);
    assert!(
        settle_while(|| window.reader().loads() == moved),
        "Enter on the row already under the cursor tore the pane's document \
         down and built the same one again — every reload is a frame of \
         unpainted WebView, which is what #749 reported seeing as black"
    );

    // ── 3. a payload arriving for the message on screen: no further load ─
    //
    // `Event::BodyLoaded` is emitted for every payload a backfill commits,
    // and the body is already local here, so the recomposed document is
    // byte-for-byte the one showing. The pane may well be *asked* to draw —
    // the parts panel's chips do change — but the person reading must not
    // have the document pulled out from under them for it.
    let before = window.reader().paints();
    for _ in 0..20 {
        wired.feeds.apply(&Event::BodyLoaded {
            account,
            message: older,
        });
    }
    assert!(
        settle_while(|| window.reader().loads() == moved),
        "an arrival that recomposes the identical document reloaded it \
         anyway: {} loads for a document that did not change",
        window.reader().loads() - moved
    );
    assert!(
        window.reader().paints() >= before,
        "the paint tally should not go backwards"
    );

    bridge.shutdown();
}

/// Write `text` as `message`'s body, as a completed fetch leaves it.
fn store_body(database: &Database, message: MessageId, text: &str) {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .set_body(
            message,
            &StoredBody {
                text: Some(text.to_owned()),
                html: None,
                headers: None,
            },
            BodyState::Full,
        )
        .expect("the body is stored");
}

/// Activate row `index`, the way Enter and a double click both arrive.
fn activate_row(window: &Window, index: u32) {
    let view = find_list_view(window.upcast_ref::<gtk::Widget>())
        .expect("the message list is built on a GtkListView");
    view.emit_by_name::<()>("activate", &[&index]);
}

fn find_list_view(widget: &gtk::Widget) -> Option<gtk::ListView> {
    if let Some(view) = widget.downcast_ref::<gtk::ListView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(found) = find_list_view(&node) {
            return Some(found);
        }
        child = node.next_sibling();
    }
    None
}
