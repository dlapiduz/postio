//! Does resting on a message in the running application actually mark it read?
//!
//! Issue #71, and the question `/issue` insists on before anything is closed:
//! can a person reach this? `postio-gtk`'s `gtk_dwell.rs` proves the pane
//! starts and cancels a clock, and `postio-session`'s own tests prove the
//! command marks a message read without touching the undo stack. Both pass in
//! a build where nothing ever calls `connect_dwelled`, and in that build
//! resting on a message does nothing at all — which is exactly the shape of
//! bug `postio-bl2` exists for and #70 shipped.
//!
//! So this starts where the application starts: a real store with real mail, a
//! real `Window`, and `feed_the_window` — the same call `run` makes. Then it
//! moves the cursor the way `j` does and reads the flag back out of SQLite.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket, and this never calls it. The `\Seen` the mark enqueues sits
//! on the queue unread by anyone, which is the local-first shape working.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This sets it before the app under test starts, which is the
// one moment it is sound. The crate's library code forbids `unsafe`.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wired, Wiring, commands, feed_the_window};
use postio_core::bridge::{Bridge, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Flag, MessageId};
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, Database, test_support};

/// Short enough that the test does not spend a real second per assertion,
/// long enough to stay distinguishable from "marked on arrival".
const DWELL: Duration = Duration::from_millis(80);

/// The delay the sweep runs under: longer than any stall a loaded machine can
/// put between a keypress and the main loop noticing it.
///
/// Not a patience dial and not a timeout — nothing ever waits this long.
/// `MessageListView::arm_dwell` re-reads the delay on every cursor report and
/// cancels whatever was armed, so a sweep under thirty seconds cannot fire a
/// dwell however long the machine stalls between keypresses. The phase becomes
/// immune by construction rather than by being quick enough.
///
/// It used to sweep at `DWELL`, which was a bet that five keypresses and their
/// repaints fit inside 80 ms of wall clock. They do on an idle workstation and
/// they do not on a loaded one, and that is the whole of `dwell_wiring`'s
/// contribution to #741: the assertion that fired was "marked read while the
/// cursor was still moving", and the cursor *had* rested — for 160 ms, because
/// the runner stalled between arming the clock and pumping the loop that reads
/// it. Reproducible on demand with a `sleep` in exactly that gap, which is how
/// this was finally pinned down.
const NEVER_WHILE_SWEEPING: Duration = Duration::from_secs(30);

/// Drive the main loop until `done`, or give up.
fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done()
}

/// Whether the store says `message` carries `\Seen`.
fn is_read(database: &Database, message: MessageId) -> bool {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .expect("a read")
        .expect("the message is still there")
        .flags
        .contains(&Flag::Seen)
}

pub fn resting_on_a_message_marks_it_read_and_sweeping_past_does_not() {
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

    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "the fixture seeded no mail");
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // Every message unread to begin with, or none of the assertions below
    // distinguish "the dwell marked it" from "it was already read".
    let inbox = {
        let connection = database.connection().expect("a connection");
        let inbox = report
            .mailbox(postio_model::MailboxRole::Inbox)
            .expect("an inbox");
        connection
            .execute(
                "UPDATE messages SET seen = 0, flags = '' WHERE mailbox_id = ?1",
                [inbox.id.get()],
            )
            .expect("the fixture writes");
        inbox.id
    };

    // A *real* bus, not a no-op handler: the whole question is whether the
    // dwell reaches a verb that writes to SQLite, so a bus that swallowed the
    // command would make this test pass while proving nothing.
    let state = SharedState::default();
    let bus = postio_app::actions::wire(
        Dispatcher::builder(),
        postio_app::actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let bus_verbs: Vec<postio_core::CommandId> = bus.wired().collect();
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

    let Wired { feeds, .. } =
        feed_the_window(&window, &wiring).expect("the seeded store has an account");
    // `feed_the_window` fills the panes; `commands::install` is what turns a
    // gesture into a command on the bus. `open_account` calls both, in this
    // order, and a test that called only the first would be asking whether a
    // gesture reaches a handler in a build where nothing wired one.
    commands::install(
        &window,
        &feeds,
        state.clone(),
        wiring.commands.clone(),
        bus_verbs.clone(),
    );
    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the list is empty, so there is nothing to rest on"
    );
    list.set_dwell_delay(DWELL);

    // ── opening the app marks nothing ────────────────────────────────────
    let untouched: Vec<MessageId> = page(&database, inbox);
    // A duration rather than `settle_until`, because there is no condition to
    // wait for: this asserts that nothing happens, and the only way to give
    // that room is to let a dwell that *would* have fired go by. Note which
    // way load pushes it -- a slow machine makes this phase weaker, never
    // redder, so it cannot be what fails intermittently.
    std::thread::sleep(DWELL * 4);
    while glib::MainContext::default().iteration(false) {}
    assert!(
        untouched.iter().all(|id| !is_read(&database, *id)),
        "launching Postio marked mail read that nobody had looked at"
    );

    // ── sweeping past rows marks none of them ────────────────────────────
    // `j` five times with no pause, which is what holding the key is. Under
    // `NEVER_WHILE_SWEEPING` the pauses this loop cannot control -- a repaint,
    // a page arriving, another session taking the CPU -- stop mattering.
    list.set_dwell_delay(NEVER_WHILE_SWEEPING);
    for _ in 0..5 {
        list.next_row();
        while glib::MainContext::default().iteration(false) {}
    }
    let swept: Vec<MessageId> = untouched.iter().take(5).copied().collect();
    assert!(
        swept.iter().all(|id| !is_read(&database, *id)),
        "a message was marked read while the cursor was still moving — the \
         unread state stops meaning anything the moment scrolling destroys it"
    );

    // ── and resting marks the one it came to rest on ─────────────────────
    // A delay short enough to wait for. `set_dwell_delay` is read at the next
    // arm rather than applied to the clock already running, so the cursor has
    // to move once more for it to take effect -- and that move also cancels
    // the thirty-second clock the sweep left armed, which is what stops it
    // marking a swept row long after this test has stopped looking.
    list.set_dwell_delay(DWELL);
    list.next_row();
    while glib::MainContext::default().iteration(false) {}
    let resting = list.cursor_id().expect("the cursor is on a row");
    assert!(
        settle_until(|| is_read(&database, resting)),
        "the cursor rested on a message and the running application never \
         marked it read. Every layer under this one is tested and passes — \
         check whether anything calls MessageListView::connect_dwelled."
    );

    bridge.shutdown();
}

/// The ids of the first page of `mailbox`, newest first.
fn page(database: &Database, mailbox: postio_model::MailboxId) -> Vec<MessageId> {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .page(&ListQuery {
            scope: ListScope::Mailbox(mailbox),
            limit: 20,
            after: None,
        })
        .expect("a page")
        .into_iter()
        .map(|row| row.id)
        .collect()
}
