#![allow(unsafe_code)]
//! Archiving or deleting the message under the cursor walks down the list
//! (#1687).
//!
//! Triage is `a a a` or `d d d`: each press takes the message under the
//! cursor out and leaves the cursor on the one that was directly below it,
//! with the rows on screen where they were apart from the gap closing. The
//! maintainer reported the opposite on `main` at d99249d8 -- the list "kind
//! of jumps to another point", and two presses in quick succession made the
//! second one fail.
//!
//! The real chain, as `run()` arranges it: the key, the bus over the local
//! store, one hub the bus emits into, and the window's subscription drained
//! into the panes by `commands::drain` -- so every event that follows the
//! optimistic removal (`MessagesRemoved`, `MessageListChanged`,
//! `MessagesChanged`) lands the way it would in the running application,
//! and the assertions are made after all of them have.
//!
//! A folder deep enough to scroll, with the cursor in the middle of it, and
//! the Unified view as well, which is the inboxes (#1692): an archive takes
//! the row out of both, and both keep the same promise.

use crate::settle_until;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wired, Wiring, commands, feed_the_window, notifications};
use postio_core::Event;
use postio_core::bridge::{Bridge, EventHub, EventStream};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ListScope;
use postio_model::ids::MessageId;
use postio_storage::seed::seed_large;
use postio_storage::{BlobStore, test_support};

/// Enough mail that the opening folder is several screens long.
const MESSAGES: usize = 400;

/// Where the cursor starts: well below the first screen, well above the end.
const START: u32 = 30;

/// Everything a case drives.
struct Triage {
    window: Window,
    /// Every event the bus and the store emitted, read by the case rather
    /// than by the panes: a rejected or failed verb says so here.
    probe: EventStream,
    _bridge: Bridge,
    _state_dir: tempfile::TempDir,
    _blobs: tempfile::TempDir,
}

async fn triage(
    scope: Option<fn(&postio_storage::seed::SeedReport) -> ListScope>,
) -> Option<Triage> {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory().await;
    let report = seed_large(&database, 11, MESSAGES).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let state = SharedState::default();
    let bus = postio_app::actions::wire(
        Dispatcher::builder(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
    let hub = EventHub::new();
    let engine = hub.sink();
    let bridge = Bridge::builder()
        .build_with_events(bus, hub.sink())
        .expect("a runtime");
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        engine,
        bridge.commands(),
    );

    let window = Window::default();
    window.set_default_size(1200, 800);
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let Wired { feeds, .. } = feed_the_window(&window, &wiring)
        .await
        .expect("the seeded store has an account");
    commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        bus_verbs,
    );
    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    let probe = hub.subscribe("probe");
    commands::drain(&window, &feeds, hub.subscribe("window"), notifier, state);

    let list = window.list();
    assert!(
        settle_until(async || list.model().n_items() > START + 20).await,
        "the opening folder never filled: {} rows",
        list.model().n_items()
    );
    if let Some(scope) = scope {
        let before = list.model().generation();
        feeds.messages.open(scope(&report));
        assert!(
            settle_until(async || list.model().generation() != before
                && list.model().n_items() > START + 20
                && list.model().peek(START + 20).is_some())
            .await,
            "the view never filled"
        );
    }

    // The cursor in the middle of a scrolled list, put there the way `j`
    // puts it anywhere: as a person's choice.
    let model = list.model();
    let _ = model.item(START + 10);
    assert!(
        settle_until(async || model.peek(START + 10).is_some()).await,
        "the rows below the cursor never landed"
    );
    let start = model.peek(START).expect("the starting row is resident");
    let laid_out = || {
        let any = std::cell::Cell::new(false);
        window
            .list()
            .each_row(|row| any.set(any.get() || row.height() > 0));
        any.get()
    };
    assert!(
        settle_until(async || laid_out()).await,
        "no row was ever laid out"
    );
    // Scrolled there first, the way a person reading down the folder is,
    // so the cursor's row is on screen and the list is well away from the
    // top.
    list.set_scroll_offset(row_height(&window) * f64::from(START - 4));
    quiesce().await;
    list.select_message(start);
    assert!(
        settle_until(async || list.cursor_id() == Some(start) && list.scroll_offset() > 0.0).await,
        "the cursor never reached the middle of a scrolled list"
    );
    quiesce().await;
    Some(Triage {
        window,
        probe,
        _bridge: bridge,
        _state_dir: state_dir,
        _blobs: directory,
    })
}

/// Let everything a verb set in motion land: the store write, the events it
/// emitted, the reads those events started, the frames that lay them out.
async fn quiesce() {
    for _ in 0..60 {
        while glib::MainContext::default().iteration(false) {}
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The height of one row as laid out, which is how far the list may
/// legitimately move when one is taken out.
fn row_height(window: &Window) -> f64 {
    let tallest = std::cell::Cell::new(0);
    window.list().each_row(|row| {
        let height = row.parent().map_or(row.height(), |item| item.height());
        tallest.set(tallest.get().max(height));
    });
    assert!(tallest.get() > 0, "no row has been laid out");
    f64::from(tallest.get())
}

/// Whatever went wrong since the last look: a rejection or an error is a
/// sentence in a toast, and a verb that fails says it here first.
fn complaints(probe: &EventStream) -> Vec<String> {
    let mut said = Vec::new();
    while let Some(event) = probe.try_next() {
        match event {
            Event::CommandRejected { reason, .. } => said.push(format!("rejected: {reason}")),
            Event::Error { message } => said.push(format!("error: {message}")),
            _ => {}
        }
    }
    said
}

/// The row directly below `message`, as the list holds it now.
fn below(window: &Window, message: MessageId) -> MessageId {
    let model = window.list().model();
    let at = model
        .position_of(message)
        .expect("the message is on screen");
    model.peek(at + 1).expect("the row below it is resident")
}

/// How a message leaves: a key a person presses, or a verb that names its
/// row itself.
#[derive(Clone, Copy)]
enum Gesture {
    Key(gdk::Key),
    /// Archive, naming the cursor's message -- what the row's own hover
    /// action sends. The cursor is not stepped off first, so the row that
    /// has the keyboard is taken out from under it.
    Named,
}

/// Take the message under the cursor out and prove the list walked down:
/// the cursor on the row that was directly below, the scroll offset where it
/// was give or take the row that left, the reading pane on the cursor's
/// message -- after everything that followed has landed too.
async fn remove_and_walk(triage: &Triage, gesture: Gesture, press: usize) {
    let window = &triage.window;
    let list = window.list();
    let model = list.model();
    let removed = list.cursor_id().expect("the cursor is on a message");
    let next = below(window, removed);
    let offset = list.scroll_offset();
    let height = row_height(window);

    match gesture {
        Gesture::Key(key) => {
            window.handle_key(key, gdk::ModifierType::empty());
        }
        Gesture::Named => window.act(postio_core::Command::Archive {
            target: postio_core::MessageTarget::Messages(vec![removed]),
        }),
    }
    assert!(
        settle_until(async || model.position_of(removed).is_none()).await,
        "press {press}: the message under the cursor never left the list: {:?}",
        complaints(&triage.probe)
    );
    // And everything that follows it: the refresh the removal started, the
    // `MessageListChanged` and `MessagesChanged` behind it.
    quiesce().await;
    assert_eq!(
        complaints(&triage.probe),
        Vec::<String>::new(),
        "press {press}: the verb complained"
    );
    assert_eq!(
        list.cursor_id(),
        Some(next),
        "press {press}: the cursor is not on the message that was directly below the \
         removed one (it is at row {} of {})",
        list.cursor().selected(),
        model.n_items()
    );
    let moved = (list.scroll_offset() - offset).abs();
    assert!(
        moved <= height + 1.0,
        "press {press}: the list jumped {moved}px (from {offset} to {}); one row is {height}px",
        list.scroll_offset()
    );
    let row = list.cursor_row().expect("the cursor's row is resident");
    let thread = row
        .thread
        .expect("a threaded view's row names its conversation");
    assert!(
        settle_until(async || window.conversation_on(thread)).await,
        "press {press}: the reading pane is not showing the cursor's message"
    );
}

pub fn archiving_and_deleting_walk_down_a_folder() {
    crate::gtk_case(async {
        let Some(triage) = triage(None).await else {
            return;
        };
        for press in 1..=3 {
            remove_and_walk(&triage, Gesture::Key(gdk::Key::a), press).await;
        }
        remove_and_walk(&triage, Gesture::Key(gdk::Key::d), 4).await;
    });
}

/// The row with the keyboard on it taken away before the cursor could step
/// off -- a hover action, or a sync that saw it archived elsewhere. GTK
/// walks the keyboard back into the list when its row goes, and the list
/// must take it to the cursor rather than to the first row, which scrolled
/// the list to the top (#1687).
pub fn a_row_taken_from_under_the_keyboard_leaves_the_list_where_it_was() {
    crate::gtk_case(async {
        let Some(triage) = triage(None).await else {
            return;
        };
        for press in 1..=2 {
            remove_and_walk(&triage, Gesture::Named, press).await;
        }
    });
}

/// Unified is the inboxes (#1692), so an archive there takes the row out of
/// the view exactly as it does out of a folder, and `a a a` walks down it:
/// the cursor on the message that was below, the row gone, the reading pane
/// following -- the folder's promise, kept by an aggregate view.
pub fn archiving_walks_down_the_unified_view() {
    crate::gtk_case(async {
        let Some(triage) = triage(Some(|_| ListScope::Unified)).await else {
            return;
        };
        for press in 1..=3 {
            remove_and_walk(&triage, Gesture::Key(gdk::Key::a), press).await;
        }

        // And faster than the round trip, which is how triage is typed: the
        // second press has to be about the row below, so the cursor has to
        // be there before the first archive's write, events and re-read are.
        let window = &triage.window;
        let list = window.list();
        let model = list.model();
        let first = list.cursor_id().expect("the cursor is on a message");
        let second = below(window, first);
        let third = below(window, second);
        window.handle_key(gdk::Key::a, gdk::ModifierType::empty());
        while glib::MainContext::default().iteration(false) {}
        window.handle_key(gdk::Key::a, gdk::ModifierType::empty());
        assert!(
            settle_until(
                async || model.position_of(first).is_none() && model.position_of(second).is_none()
            )
            .await,
            "two quick presses in Unified did not take two messages out: {:?}",
            complaints(&triage.probe)
        );
        quiesce().await;
        assert_eq!(
            complaints(&triage.probe),
            Vec::<String>::new(),
            "a quick second press in Unified complained"
        );
        assert_eq!(
            list.cursor_id(),
            Some(third),
            "the cursor is not on the message below the two archived"
        );
    });
}

/// Two presses faster than the store, the events and the refresh: the
/// second acts on the message the first left the cursor on, and neither
/// complains.
pub fn two_presses_back_to_back_take_two_messages() {
    crate::gtk_case(async {
        let Some(triage) = triage(None).await else {
            return;
        };
        let window = &triage.window;
        let list = window.list();
        let model = list.model();

        // In one turn of the main loop.
        let first = list.cursor_id().expect("the cursor is on a message");
        let second = below(window, first);
        let third = below(window, second);
        window.handle_key(gdk::Key::d, gdk::ModifierType::empty());
        window.handle_key(gdk::Key::d, gdk::ModifierType::empty());
        assert!(
            settle_until(
                async || model.position_of(first).is_none() && model.position_of(second).is_none()
            )
            .await,
            "two presses in one turn did not take two messages out: {:?}",
            complaints(&triage.probe)
        );
        quiesce().await;
        assert_eq!(
            complaints(&triage.probe),
            Vec::<String>::new(),
            "a press in the same turn as the last complained"
        );
        assert_eq!(
            list.cursor_id(),
            Some(third),
            "the cursor is not on the message below the two deleted"
        );

        // And with a moment between them, shorter than the round trip.
        let first = third;
        let second = below(window, first);
        let third = below(window, second);
        window.handle_key(gdk::Key::a, gdk::ModifierType::empty());
        while glib::MainContext::default().iteration(false) {}
        window.handle_key(gdk::Key::a, gdk::ModifierType::empty());
        assert!(
            settle_until(
                async || model.position_of(first).is_none() && model.position_of(second).is_none()
            )
            .await,
            "two quick presses did not take two messages out: {:?}",
            complaints(&triage.probe)
        );
        quiesce().await;
        assert_eq!(
            complaints(&triage.probe),
            Vec::<String>::new(),
            "a quick second press complained"
        );
        assert_eq!(
            list.cursor_id(),
            Some(third),
            "the cursor is not on the message below the two archived"
        );
        let thread = list
            .cursor_row()
            .and_then(|row| row.thread)
            .expect("the cursor's row names its conversation");
        assert!(
            settle_until(async || window.conversation_on(thread)).await,
            "the reading pane is not showing the cursor's message"
        );
    });
}
