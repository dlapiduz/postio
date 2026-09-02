//! Opening a search result actually opens it (#767).
//!
//! `Ret` on a previewed result — and the preview's own `Open` button — emit
//! `Preview::open`, and `postio-app`'s `install_open` turns that into
//! `Command::OpenMessage`. Nothing answered that command. It is not in
//! `postio_session::actions::WIRED`, not in `refresh::wire`, and the window
//! did not handle it either, so the send reached the dispatcher and came back
//! as `Event::CommandRejected`: the one gesture whose entire purpose is
//! "open this" did nothing at all.
//!
//! `command_wiring.rs` had already caught the shape of it — `OpenMessage` sat
//! in that sweep's `KNOWN_ORPHANS` citing this issue — but a sweep can only
//! say "nothing answers this id". What it cannot say is what a person sees,
//! which is why #767 asked for the real path rather than another trace.
//!
//! So this drives the gesture and asserts the outcome: search, focus a
//! result, `Preview::open()`, and the reading pane is showing *that message*.
//! Deliberately not `window.show_message(...)` — `gtk_reader_pane_owner.rs`
//! documents this flow and calls that directly, which is why it stayed green
//! through the whole bug (`postio-bl2`, the shape CLAUDE.md warns about).
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::{gdk, glib};
use postio_app::{Wired, commands, feed_the_window, notifications};
use postio_core::bridge::{Bridge, EventHub, handler_fn};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::repository::MessageRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// Matches the seeded corpus: every fixture address is on a reserved domain.
const QUERY: &str = "example.com";

pub fn opening_a_previewed_result_shows_it_in_the_reading_pane() {
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
    let report = seed_small(&database, 11);
    let subject_of = |message| {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .get(message)
            .expect("a read")
            .expect("the previewed message is in the store")
            .subject
            .unwrap_or_default()
    };
    assert!(report.message_count > 0, "the fixture seeded no mail");
    ensure_search_index(&database).expect("the index is part of opening the store");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A real bridge, so a command that nothing answers is rejected by the
    // real dispatcher rather than swallowed by a stub that accepts anything.
    //
    // And `run`'s own event arrangement, because the results only reach the
    // list through `Event::SearchResults` -> `Feeds::apply`: one hub the
    // search emits into, drained into the panes. Without the drain the box
    // finds hits, the preview shows one, and the list never leaves the
    // folder -- so there would be nothing for the open to land on, and the
    // test would fail for a reason that is not the bug.
    let hub = EventHub::new();
    let engine = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(handler_fn(|_, _| async {}), hub.sink())
        .expect("a runtime");
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        engine,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // The same call `run` makes, and the `View` it returns rather than a
    // second `search::install` — two installs answer into a view the test
    // cannot see.
    let Wired { feeds, search } =
        feed_the_window(&window, &wiring).expect("the store has an account");
    let view = search.expect("search installed");
    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    commands::drain(&window, &feeds, hub.subscribe("window"), notifier);

    // ── search, and let the preview settle on a result ───────────────────
    let finder = window.finder();
    finder.open(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: QUERY.to_owned(),
    });
    finder
        .live()
        .expect("the box has a live readout while searching")
        .flush();

    // The hits have to be in the list before anything can be opened out of
    // it -- that switch is what `search_results.rs` proves separately.
    let showing = settle_until(|| feeds.messages.showing_results());
    assert!(
        showing,
        "the box found hits and the list is still showing the folder, so \
         there is nothing here for an open to land on"
    );
    let previewed = settle_until(|| view.preview().focused().is_some());
    assert!(
        previewed,
        "the search found nothing to preview, so this test could not fail"
    );
    let message = view
        .preview()
        .focused()
        .expect("the preview is showing a result");

    // Cleared first, so what follows is about *opening* rather than about
    // whatever the pane happened to be showing already.
    window.clear_reader();
    assert!(!window.reading(), "the pane was just cleared");

    // ── the gesture: `Ret` on the result, and the Open button ────────────
    // `Preview::open` is what both are wired to. Everything past this point
    // is the application's own wiring answering — or, before #767, not.
    view.preview().open();

    assert!(
        settle_until(|| window.reading()),
        "opening a previewed search result left the reading pane empty. The \
         gesture emits `Preview::open`, which becomes `Command::OpenMessage` \
         — and nothing answers that, so the dispatcher rejects it and the one \
         verb whose whole purpose is \"open this\" does nothing."
    );
    // Which message, read the way a person reads it: the subject on the
    // reader's own header. Asserting the pane merely *filled* would pass on
    // opening the wrong message, which is the failure a command carrying an
    // id can actually produce.
    let expected = subject_of(message);
    assert!(
        !expected.trim().is_empty(),
        "the previewed fixture has no subject, so this assertion would hold \
         for any message"
    );
    assert!(
        settle_until(|| window.reader().header().subject_label() == expected),
        "the pane opened on a different message than the one previewed: the \
         header reads {:?}, and the result was {expected:?}",
        window.reader().header().subject_label()
    );

    bridge.shutdown();
}
