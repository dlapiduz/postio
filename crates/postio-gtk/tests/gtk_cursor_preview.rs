//! The cursor announces where it is, so the reading pane can follow it.
//!
//! Issue #70, Cause B: the pane was fed by `connect_activated`, which is
//! Enter or a double click. Moving the cursor with `j`/`k` changed what the
//! user was looking at and told nobody, so the pane kept showing the last
//! message somebody had pressed Enter on -- or, on a fresh window, nothing
//! at all. The maintainer settled the design: the preview follows the
//! cursor, and nothing waits for Return.
//!
//! `connect_activated` cannot be reused for this. In this pane the cursor and
//! the selection are deliberately two different facts (`gtk_selection.rs`),
//! and activation is a third: `j` moves the cursor without selecting and
//! without activating. So the pane needs a signal of its own.
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

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
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
fn the_cursor_reports_every_row_it_lands_on() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = MessageListView::new();

    // Subscribed before any row arrives, which is the order `reading.rs`
    // wires it in: handlers at startup, rows when the first page lands.
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

    // ── the autoselect is not somebody looking at a message ─────────────
    // `SingleSelection` puts the cursor on row 0 as soon as the model has
    // rows. Nobody chose that, so the reading pane must not fill on startup
    // -- and once #71's dwell timer exists, filling here would mark the
    // newest message read for the sole reason that the app was opened.
    pane.model().set_source(Rc::new(Pages));
    pump();
    assert!(
        seen.borrow().is_empty(),
        "the autoselect reported a landing nobody asked for"
    );

    // ── a real move onto a page that has not arrived reports nothing yet ──
    // The rows are still placeholders: `set_source` sizes the model before
    // `deliver` fills it. There is no message to show, so there is nothing
    // to say.
    pane.first_row();
    pump();
    assert!(
        seen.borrow().is_empty(),
        "a placeholder is not a message and must not be reported as one"
    );

    // ── and reports it the moment the page lands ─────────────────────────
    // This is what the `items_changed` hookup is for: the cursor was already
    // where it belongs, so `notify::selected` has been and gone.
    pane.model().deliver(0, (0..ROWS).map(row).collect());
    pump();
    assert_eq!(
        *seen.borrow(),
        vec![MessageId::new(1)],
        "the page arrived under a cursor the user had moved, and the reader \
         was never told"
    );

    // ── `j` down the list reports each row, in order ─────────────────────
    // The pane has to hear about every landing, not just the last one: the
    // reader starts a store read per message and the late-answer guard in
    // `reading.rs` depends on being told each time which one is current.
    seen.borrow_mut().clear();
    pane.next_row();
    pane.next_row();
    pump();
    assert_eq!(
        *seen.borrow(),
        vec![MessageId::new(2), MessageId::new(3)],
        "every row the cursor landed on should have been reported, in order"
    );

    // ── `k` back up reports too ──────────────────────────────────────────
    seen.borrow_mut().clear();
    pane.prev_row();
    pump();
    assert_eq!(
        *seen.borrow(),
        vec![MessageId::new(2)],
        "moving back up is a move like any other"
    );

    // ── a move that goes nowhere says nothing ────────────────────────────
    // `k` on the first row is a no-op, and a no-op that re-reported would
    // make the reader re-read the store for a message already on screen.
    pane.first_row();
    pump();
    seen.borrow_mut().clear();
    pane.prev_row();
    pump();
    assert!(
        seen.borrow().is_empty(),
        "the cursor did not move, so there was nothing to report"
    );

    // ── selecting is not moving ──────────────────────────────────────────
    // `x` toggles the selection under the cursor. The cursor stays put, so
    // the pane must not be told to change what it is showing.
    seen.borrow_mut().clear();
    pane.toggle_cursor_row();
    pump();
    assert!(
        seen.borrow().is_empty(),
        "toggling the selection moved no cursor and must feed the reader nothing"
    );

    // ── a row repainting under the cursor is not a landing ───────────────
    // `update_row` is how a flag toggle, a `\Seen` change or any other sync
    // edit reaches the list, and it fires `items_changed` exactly like a page
    // arriving does. The message under the cursor has not changed, so the
    // reader must not be told to show it again: that would re-run a store
    // read on every incoming flag, and once the dwell timer of #71 exists it
    // would restart that too — marking mail read because a sync touched it.
    assert_eq!(
        pane.cursor().selected(),
        0,
        "the cursor is on the first row"
    );
    seen.borrow_mut().clear();
    let mut repainted = row(0);
    repainted.flagged = true;
    assert!(
        pane.model().update_row(repainted),
        "the row under the cursor should have been resident to repaint"
    );
    pump();
    assert!(
        seen.borrow().is_empty(),
        "a repaint of the row already under the cursor is not a landing"
    );

    window.close();
}
