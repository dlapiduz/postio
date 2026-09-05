//! The first-run keyboard orientation, in an application that has one.
//!
//! ADR 0012 Q4-Q6, and the acceptance of #288: it appears exactly once,
//! after the first successful sync; dismissing it or running a command
//! retires it permanently; and a later run of the same store never shows it
//! again.
//!
//! # Why this cannot be a widget test
//!
//! `gtk_suite/gtk_orientation.rs` proves the strip draws the keys it is
//! handed, and `postio_app::orientation`'s unit tests prove the ordering
//! rules. Both pass in an application where nothing ever shows the strip —
//! which is `postio-bl2`, and which is what happened to the Reader: built,
//! tested, and mounted nowhere. So every assertion here is about what a
//! person would see in the window, driven through the same
//! `feed_the_window` the binary calls, and the dismissal goes through the
//! real button found in the real widget tree rather than through a method
//! that stands in for one.
//!
//! Nothing here touches the network: the store is seeded locally and the
//! sync engine is never started. `Event::SyncProgress` is what the engine
//! would emit at the end of a pass, applied directly.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::{settle, settle_until, settle_while};
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::Event;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::AccountId;
use postio_session::Wiring;
use postio_storage::seed::{SeedReport, seed_small};
use postio_storage::{BlobStore, Database, test_support};

/// A store with mail in it, and the pieces every window here needs.
struct World {
    database: Database,
    blobs: BlobStore,
    seeded: SeedReport,
    /// Kept alive: dropping it removes the directory the blobs live in.
    _directory: tempfile::TempDir,
}

fn world() -> World {
    let database = test_support::memory();
    let seeded = seed_small(&database, 11);
    assert!(
        seeded.message_count > 0,
        "the fixture seeded no mail, so nothing below could be navigated"
    );
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    World {
        database,
        blobs,
        seeded,
        _directory: directory,
    }
}

/// Start the application over `world`'s store, the way `run` starts it.
///
/// Returned rather than held: a second call is a second run of the same
/// installation, which is how "never shown again" is asked.
fn launch(world: &World) -> (Window, postio_gtk::feed::Feeds, Bridge) {
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        world.database.clone(),
        world.blobs.clone(),
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
    (window, feeds, bridge)
}

/// What the sync engine emits when a list pass reaches its own total.
fn sync_finished(account: AccountId) -> Event {
    Event::SyncProgress {
        account,
        done: 4,
        total: 4,
    }
}

/// The strip's own dismiss button, found where a person would click it.
fn got_it(window: &Window) -> Option<gtk::Button> {
    fn walk(widget: &gtk::Widget, found: &mut Option<gtk::Button>) {
        if found.is_none()
            && widget.has_css_class("postio-orientation-dismiss")
            && let Ok(button) = widget.clone().downcast::<gtk::Button>()
        {
            *found = Some(button);
        }
        let mut child = widget.first_child();
        while let Some(node) = child {
            walk(&node, found);
            child = node.next_sibling();
        }
    }
    let mut found = None;
    walk(window.upcast_ref::<gtk::Widget>(), &mut found);
    found
}

fn display() -> bool {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    true
}

pub fn the_first_sync_shows_it_and_got_it_ends_it_for_every_later_run() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    if !display() {
        return;
    }

    let world = world();
    let account = world.seeded.account.id;
    let (window, feeds, bridge) = launch(&world);

    // ── before a sync there is nothing to navigate ──────────────────────
    // ADR 0012 Q4: "press j and k to move between messages" refers to
    // nothing until there is mail on screen, so the strip waits.
    assert!(
        settle_while(|| !window.orientation().is_visible()),
        "the orientation appeared before any sync had finished"
    );

    // ── the first pass finishes ─────────────────────────────────────────
    feeds.apply(&sync_finished(account));
    assert!(
        settle_until(|| window.orientation().is_visible()),
        "the first sync finished and the window never taught anybody the \
         keyboard. The strip is built and it is mounted; nothing showed it."
    );

    // ── and a person can put it away ────────────────────────────────────
    let button = got_it(&window).expect("the strip offers a way out of itself");
    assert!(
        button.is_visible() && button.is_sensitive(),
        "the dismissal is on screen but cannot be pressed"
    );
    button.emit_clicked();
    assert!(
        settle_until(|| !window.orientation().is_visible()),
        "\"Got it\" reported into nothing: the button is drawn, it is \
         clickable, and the strip is still there"
    );
    bridge.shutdown();

    // ── the next run of the same installation ───────────────────────────
    // The acceptance is "never again", which is a claim about runs and not
    // about sessions -- so this is a second window over the same store,
    // told the same thing by the same engine.
    let (second, feeds, bridge) = launch(&world);
    feeds.apply(&sync_finished(account));
    assert!(
        settle_while(|| !second.orientation().is_visible()),
        "a dismissed orientation came back on the next run: whatever was \
         remembered did not outlive the window that remembered it"
    );
    bridge.shutdown();
}

pub fn a_command_retires_it_even_when_it_was_never_on_screen() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    if !display() {
        return;
    }

    // ── the control ─────────────────────────────────────────────────────
    // A store nobody has touched, taken to the same point: without this the
    // assertions below would pass in an application that never shows the
    // strip at all, which is the failure they exist to catch.
    let untouched = world();
    let (control, feeds, bridge) = launch(&untouched);
    feeds.apply(&sync_finished(untouched.seeded.account.id));
    assert!(
        settle_until(|| control.orientation().is_visible()),
        "a first sync on an untouched store does not show the orientation,          so nothing below can distinguish \"suppressed\" from \"never shown\""
    );
    bridge.shutdown();

    // ── and now the same thing, with a keystroke first ──────────────────
    let world = world();
    let account = world.seeded.account.id;
    let (window, feeds, bridge) = launch(&world);

    // Somebody who presses `j` before the first sync has already
    // demonstrated the thing the strip exists to teach them (ADR 0012 Q6),
    // so it must never appear -- not on this pass, and not on any later
    // run. `j` rather than a synthetic dispatch, because the criterion is
    // about a command the user actually ran.
    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "no rows, so `j` would have nothing to move between"
    );
    window.handle_key(gdk::Key::j, gdk::ModifierType::empty());
    settle();

    feeds.apply(&sync_finished(account));
    assert!(
        settle_while(|| !window.orientation().is_visible()),
        "the user moved through their mail with the keyboard and Postio \
         then offered to explain the keyboard"
    );
    bridge.shutdown();

    let (second, feeds, bridge) = launch(&world);
    feeds.apply(&sync_finished(account));
    assert!(
        settle_while(|| !second.orientation().is_visible()),
        "retiring it without showing it was not written down: it came back \
         on the next run"
    );
    bridge.shutdown();
}
