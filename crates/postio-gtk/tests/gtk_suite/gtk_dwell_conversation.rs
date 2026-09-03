//! Opening a conversation stops the list's read-clock.
//!
//! #797. Two dwell timers are live once a thread row is opened: the
//! conversation pane's, armed on the focused message, which is what ADR 0015
//! Q4 asks for; and the list's, armed when the cursor landed on the row.
//! A thread row's id is its *representative* — the newest message in that
//! folder — so letting the list's clock run marks a message read that focus
//! may never reach. "Opened the thread, all six read", which Q4 forbids in
//! those words.
//!
//! # Why this is not covered by `app_suite/thread_dwell.rs`
//!
//! That suite has the right assertion and cannot depend on it. It shortens
//! the *conversation's* dwell to 80ms and never touches the list's, which
//! therefore runs at the real one second — while the negative assertion is
//! made about 320ms in. The list's clock had not fired yet whatever the code
//! did, so the assertion passed on a developer's machine, and only when a
//! loaded runner reordered the two did it catch anything. #797's own summary
//! of that: "it passes on a developer's machine and failed on a runner …
//! that is timing, not luck of the draw".
//!
//! So this arms the list's clock with a delay short enough to fire, opens a
//! conversation, and waits past it. It fails because the clock was not
//! stopped, not because of where a machine happened to be.
//!
//! # The other half: the conversation's own clock
//!
//! #945 asks for an audit of every place "what the reader is showing" can
//! change. The list's clock is stopped at three of them; the conversation's
//! is stopped at none, and the same argument applies to it word for word.
//! `a_single_message_taking_the_pane_stops_the_conversations_clock` is the
//! one of those a test can drive directly.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{MessageId, ThreadId};

const ROWS: u32 = 4;

/// Short enough that waiting past it several times is still a fast test,
/// long enough to be distinguishable from "fired immediately".
const DWELL: Duration = Duration::from_millis(60);

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
}

/// A row standing for a conversation, the way the feed builds one.
fn row(position: u32) -> Row {
    Row {
        id: MessageId::new(position as i64 + 1),
        thread: Some(ThreadId::new(position as i64 + 1)),
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
        thread_count: 3,
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
}

fn settle() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

/// Drive the loop until `done`, for the *positive* controls.
///
/// A fixed `wait` is right for the negative assertions below — you cannot
/// wait for something not to happen, so the duration is part of what they
/// mean — and wrong for the controls, which are waiting for a timer that
/// will fire. Written as a duration first, this file failed twice on a
/// loaded machine and passed once an `eprintln!` slowed it down, which is
/// the whole of why CLAUDE.md says to wait on conditions.
fn settle_until(done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    let context = gtk::glib::MainContext::default();
    while !done() && std::time::Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn opening_a_conversation_stops_the_lists_read_clock() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let list = window.list();
    list.set_dwell_delay(DWELL);

    let dwelled: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    list.connect_dwelled({
        let dwelled = dwelled.clone();
        move |message| dwelled.borrow_mut().push(message)
    });

    list.model().set_source(Rc::new(Pages));
    list.model().deliver(0, (0..ROWS).map(row).collect());
    settle();

    // ── the control: the clock does run, and does fire ───────────────────
    // Without this the assertion below is unfalsifiable. A list whose dwell
    // never armed at all reports nothing for the same reason a list whose
    // dwell was correctly cancelled reports nothing, and this file would
    // pass on a build where the whole mechanism had been deleted.
    //
    // `next_row` rather than `first_row`: the autoselect already parked the
    // cursor on row 0 and the pane dedupes on the id, so moving *to* row 0
    // is not a landing.
    list.next_row();
    settle_until(|| !dwelled.borrow().is_empty());
    assert_eq!(
        *dwelled.borrow(),
        vec![MessageId::new(2)],
        "resting on a row did not start or fire the list's clock, so nothing \
         below this could fail"
    );
    dwelled.borrow_mut().clear();

    // ── and opening a conversation stops it ──────────────────────────────
    // The landing arms the clock for row 3's representative; the
    // conversation takes the pane before it runs out. `show_conversation` is
    // the single point both routes in pass through — the drill-in column and
    // #755's open-from-the-list — and the moment the row stops being what is
    // in front of the reader.
    list.next_row();
    window.show_conversation(vec![row(9), row(10)]);
    wait(DWELL * 4);

    assert!(
        dwelled.borrow().is_empty(),
        "the list's clock ran on while a conversation was open, so the row's \
         representative was marked read without focus ever reaching it — \
         'opened the thread, all six read', which ADR 0015 Q4 forbids: {:?}",
        dwelled.borrow()
    );

    window.destroy();
}

pub fn a_single_message_taking_the_pane_stops_the_conversations_clock() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let pane = window.conversation();
    pane.set_dwell_delay(DWELL);

    let dwelled: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    pane.connect_dwelled({
        let dwelled = dwelled.clone();
        move |message| dwelled.borrow_mut().push(message)
    });

    window.show_conversation(vec![row(0), row(1), row(2)]);
    settle();

    // ── the control, for the reason the case above has one ───────────────
    pane.focus_message(MessageId::new(2));
    settle_until(|| !dwelled.borrow().is_empty());
    assert_eq!(
        *dwelled.borrow(),
        vec![MessageId::new(2)],
        "resting on a focused message did not start or fire the \
         conversation's clock, so nothing below this could fail"
    );
    dwelled.borrow_mut().clear();

    // ── and a single message taking the pane stops it ────────────────────
    // `show_message` says so itself: "a single message takes the pane back
    // from a conversation (#755): the cursor moved to a row that is not one,
    // so the stack would be showing mail the user has left". Mail the user
    // has left is not mail in front of them, and the clock that measures
    // exactly that has to stop -- the same argument #797 made for the list's
    // clock, which is stopped here and was not.
    pane.focus_message(MessageId::new(3));
    window.show_message(
        &postio_model::MessageBody {
            text: Some("a different message entirely".to_owned()),
            html: None,
        },
        Some("ada@example.com"),
    );
    wait(DWELL * 4);

    assert!(
        dwelled.borrow().is_empty(),
        "the conversation's clock ran on after a single message took the \
         pane, marking read a message the reader had already left: {:?}",
        dwelled.borrow()
    );

    window.destroy();
}
