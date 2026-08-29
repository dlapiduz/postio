//! The window's one subscription actually reaches the panes.
//!
//! The hub (#176, ADR 0013) replaced two channels collected by hand in a
//! `Rc<RefCell<Vec<Option<EventStream>>>>` with a single subscription. Every
//! layer under that is unit-tested and every one of those tests passes
//! whether or not the window is subscribed to the hub the producers write
//! into — which is the `postio-bl2` shape exactly: the search UI was built,
//! tested and fed by nothing, and the Reader was never mounted at all.
//!
//! So this asserts the only thing that could have failed: a producer emits,
//! and *the window changes*. Not "the hub delivered", not "drain was called".
//!
//! `Event::MessagesRemoved` is the event under test because its effect is
//! visible through public API — `commands::apply` clears the selection, so
//! rows that left the mailbox cannot stay selected and have the next verb
//! aimed at them.
//!
//! One test function: GTK is initialised once, per process, from one thread.
//! Nothing here dials anything — `feed_the_window` reads the local store and
//! `start_syncing`, the half that opens a socket, is never called.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{commands, feed_the_window, notifications};
use postio_core::Event;
use postio_core::bridge::{Bridge, EventHub, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

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

pub fn an_event_from_a_producer_that_is_not_the_bus_reaches_the_panes() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // ── exactly `run`'s arrangement ─────────────────────────────────────
    // One hub. The bus emits into it through the bridge; the sync engine
    // holds a sink of its own on the same hub; the window subscribes once.
    let hub = EventHub::new();
    let engine = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(handler_fn(|_, _| async {}), hub.sink())
        .expect("a runtime");
    let wiring = Wiring::new(
        database,
        blobs,
        bridge.handle(),
        engine.clone(),
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    // The one line `open_account` runs, over the one subscription it now
    // takes instead of a stream per producer.
    commands::drain(&window, &feeds, hub.subscribe("window"), notifier);

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the window was fed {} messages and the list is empty, so nothing \
         below could fail",
        report.message_count
    );
    let first = list.model().peek(0).expect("a first row");
    // The multi-select set, which is what `commands::apply` clears --
    // `select_message` moves the cursor, which is a different thing.
    list.select_all();
    assert!(
        !list.selection().is_empty(),
        "nothing selected, so clearing it would prove nothing"
    );

    // ── the assertion ───────────────────────────────────────────────────
    // The sync engine is not a command handler and is never handed a sink by
    // the bridge; before the hub it had a channel of its own that the window
    // had to be given separately. If that wiring is wrong, mail arrives and
    // the panes never hear about it.
    engine.emit(Event::MessagesRemoved {
        account: report.account.id,
        mailbox: feeds
            .messages
            .mailbox()
            .expect("the list is showing a mailbox"),
        messages: vec![first],
    });

    assert!(
        settle_until(|| list.selection().is_empty()),
        "an event emitted by a producer reached no pane. The hub delivered it \
         and every layer's own tests pass; what is missing is between them — \
         the window is subscribed to something the producers do not write to."
    );

    bridge.shutdown();
}
