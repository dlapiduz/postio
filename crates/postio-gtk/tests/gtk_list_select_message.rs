//! Putting the cursor on a specific message from outside the list.
//!
//! `postio-du6` shipped notifications that could only raise the window; a
//! click could not say *which* message it was about, because nothing in
//! `postio-gtk` could put the cursor on a row it did not already have
//! resident. This is that capability's own test — `gtk_window_open_message.rs`
//! covers the window-level API that calls it after switching folders.
//!
//! One `#[test]`, like the rest of `gtk_*`: a window costs seconds to
//! realise, and GTK may be initialised once per process (#41).

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::list_view::MessageListView;
use postio_gtk::{fonts, style};
use postio_model::ids::MessageId;

const ROWS: u32 = 6;

/// Records every page it was asked for, and answers only when told to.
struct Pages {
    requested: RefCell<Vec<u32>>,
}

impl Pages {
    fn new() -> Rc<Self> {
        Rc::new(Pages {
            requested: RefCell::new(Vec::new()),
        })
    }
}

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, page: u32) {
        self.requested.borrow_mut().push(page);
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
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
    }
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

#[test]
fn select_message_lands_the_cursor_once_the_row_is_resident() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = MessageListView::new();
    let seen: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    pane.connect_cursor_moved({
        let seen = seen.clone();
        move |row| seen.borrow_mut().push(row.id)
    });

    let window = gtk::Window::new();
    window.set_default_size(404, 600);
    window.set_child(Some(&pane));
    window.present();
    pump();

    let source = Pages::new();
    pane.model().set_source(source.clone());
    pump();

    // ── nothing is resident yet: select_message has to ask for it ─────────
    seen.borrow_mut().clear();
    pane.select_message(MessageId::new(4));
    pump();
    assert!(
        source.requested.borrow().contains(&0),
        "asking for a message that is not resident should ask for the page \
         that would hold it, the same way a view scrolling there would"
    );
    assert!(
        seen.borrow().is_empty(),
        "the row has not arrived yet, so there is nothing to report"
    );

    // ── and lands the moment the page answers ──────────────────────────────
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();
    assert_eq!(
        pane.cursor_id(),
        Some(MessageId::new(4)),
        "the cursor should have moved onto the message once its row arrived"
    );
    assert_eq!(
        *seen.borrow(),
        vec![MessageId::new(4)],
        "the reading pane follows the cursor, so it should have heard about \
         this landing too"
    );

    // ── an already-resident message is selected immediately ────────────────
    // No second page request: `position_of` already has the answer.
    let requests_before = source.requested.borrow().len();
    pane.select_message(MessageId::new(3));
    pump();
    assert_eq!(
        pane.cursor_id(),
        Some(MessageId::new(3)),
        "the row was already there"
    );
    assert_eq!(
        source.requested.borrow().len(),
        requests_before,
        "a resident row needs no page request"
    );

    // ── a message that never shows up leaves the cursor where it was ───────
    // A notification can outlive its message -- moved, deleted, or the click
    // races something else that changed the mailbox. Asking for one that is
    // not in the page it gets back must not crash or hang retrying.
    pane.select_message(MessageId::new(999));
    pump();
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();
    assert_eq!(
        pane.cursor_id(),
        Some(MessageId::new(3)),
        "a message that never arrived should not have disturbed the cursor"
    );

    window.close();
}
