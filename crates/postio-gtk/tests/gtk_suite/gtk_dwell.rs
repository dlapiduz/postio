//! The cursor resting on a message is what marks it read — not passing over it.
//!
//! Issue #71. Once the reading pane followed the cursor (#70), "the message
//! under the cursor" became "the message on screen", and marking *that* read
//! is what every mail client with a preview pane does. Marking it on arrival
//! is what this file exists to prevent: scrolling from one end of a mailbox to
//! the other passes over every message in between, and marking all of them
//! destroys the unread state as a signal — the one thing it is for.
//!
//! So the pane starts a clock on each landing and reports only if the cursor
//! is still there when it runs out. What the report *means* — a SQLite write,
//! a `\Seen` on the queue — is the composition root's business; this pane
//! cannot reach the store and does not try.
//!
//! The delay is shortened here with [`MessageListView::set_dwell_delay`]. A
//! test that waited out the real [`postio_gtk::list_view::DWELL_TO_READ`] per
//! assertion would spend seconds proving something about ordering, and the
//! thing worth proving is which landings fire and which do not.
//!
//! One `#[test]`, like the rest of `gtk_*`: GTK may be initialised once per
//! process (#41).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::list_view::MessageListView;
use postio_gtk::{fonts, style};
use postio_model::ids::MessageId;

const ROWS: u32 = 6;

/// Long enough to be distinguishable from "immediately", short enough that
/// waiting it out several times is still a fast test.
const DWELL: Duration = Duration::from_millis(60);

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
}

/// Sixty-four turns, deliberately, and **not** `crate::pump`.
///
/// This file proves a message is *not* marked read while the cursor is
/// moving. That is a negative assertion, so the amount of settling is part
/// of what is under test: turn the loop long enough and the dwell timer
/// fires, the message is marked, and the test fails for the reason it
/// exists to catch.
///
/// The shared `pump` is 200 drains because for a *positive* assertion more
/// settling is never worse. Here it is worse, which is the exception that
/// argument has (#842).
fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

fn row(position: u32) -> Row {
    Row {
        id: MessageId::new(position as i64 + 1),
        thread: None,
        from: Some(postio_model::address::EmailAddress::new(
            Some("Ada Lovelace"),
            "ada@example.com",
        )),
        subject: Some(format!("Note {position}")),
        preview: Some("…".into()),
        received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 0, 0).unwrap(),
        seen: false,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
        participants: Vec::new(),
    }
}

/// Drive the main loop for `time`, so a `glib` timeout actually gets to fire.
fn wait(time: Duration) {
    let deadline = std::time::Instant::now() + time;
    let context = gtk::glib::MainContext::default();
    while std::time::Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
    pump();
}

pub fn a_message_is_marked_read_by_resting_on_it_not_by_passing_over_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = MessageListView::new();
    pane.set_dwell_delay(DWELL);

    let dwelled: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    pane.connect_dwelled({
        let dwelled = dwelled.clone();
        move |message| dwelled.borrow_mut().push(message)
    });

    let window = gtk::Window::new();
    window.set_default_size(404, 600);
    window.set_child(Some(&pane));
    window.present();
    pump();

    pane.model().set_source(Rc::new(Pages));
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();

    // ── opening the app marks nothing ────────────────────────────────────
    // `SingleSelection` autoselects row 0 as soon as the model has rows.
    // Nobody chose it, so no clock starts — otherwise launching Postio would
    // mark the newest message read for that reason alone, which is the unread
    // signal destroying itself.
    wait(DWELL * 3);
    assert!(
        dwelled.borrow().is_empty(),
        "the autoselect marked a message read that nobody had looked at: {:?}",
        dwelled.borrow()
    );

    // ── resting on a row marks it ────────────────────────────────────────
    //
    // `next_row` rather than `first_row`: the autoselect already parked the
    // cursor on row 0 and `report_cursor` dedupes on the id, so moving *to*
    // row 0 is not a landing. The first real one is row 1 — which is also
    // why opening the app and pressing nothing leaves the newest message
    // unread, asserted above.
    pane.next_row();
    pump();
    assert!(
        dwelled.borrow().is_empty(),
        "marked on arrival rather than on dwell"
    );
    wait(DWELL * 3);
    assert_eq!(
        *dwelled.borrow(),
        vec![MessageId::new(2)],
        "the cursor rested on a row and it was never marked read"
    );

    // ── sweeping past rows marks none of them ────────────────────────────
    // The whole point. Each move cancels the clock the last one started, so a
    // held `j` through a mailbox leaves every row it passed over unread.
    dwelled.borrow_mut().clear();
    for _ in 0..4 {
        pane.next_row();
        pump();
    }
    assert!(
        dwelled.borrow().is_empty(),
        "a message was marked read while the cursor was still moving: {:?}",
        dwelled.borrow()
    );

    // ── and only the row it came to rest on is marked ────────────────────
    wait(DWELL * 3);
    assert_eq!(
        *dwelled.borrow(),
        vec![MessageId::new(6)],
        "exactly the row the sweep ended on should be marked, and only it"
    );

    // ── the same row landed on twice is not marked twice ─────────────────
    // A repaint under the cursor — a flag change, a page arriving, a sync
    // touching the row — is not a new landing, and must not restart the
    // clock either. `report_cursor` dedupes on the id; this is the assertion
    // that says so from outside.
    dwelled.borrow_mut().clear();
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();
    wait(DWELL * 3);
    assert!(
        dwelled.borrow().is_empty(),
        "a repaint under a resting cursor marked the message again: {:?}",
        dwelled.borrow()
    );

    // ── cancelling stops a dwell in flight ───────────────────────────────
    // What the window calls when it loses focus, or when the composer takes
    // the reading pane: the message stopped being in front of a person, so
    // the clock it started means nothing any more.
    dwelled.borrow_mut().clear();
    pane.prev_row();
    pump();
    pane.cancel_dwell();
    wait(DWELL * 3);
    assert!(
        dwelled.borrow().is_empty(),
        "a cancelled dwell fired anyway, so leaving Postio open on a message \
         still marks it read: {:?}",
        dwelled.borrow()
    );

    // ── and the next landing arms a fresh one ────────────────────────────
    // Cancelling is not switching the feature off.
    pane.prev_row();
    pump();
    wait(DWELL * 3);
    assert_eq!(
        *dwelled.borrow(),
        vec![MessageId::new(4)],
        "cancelling one dwell stopped every dwell after it"
    );

    window.destroy();
}
