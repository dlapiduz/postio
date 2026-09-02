//! New mail reveals itself when the inbox is at the top, and disturbs
//! nothing when it is not — #750.
//!
//! `MessageList::inserted_at_top` is deliberately an insertion, not a reset
//! (#72), but `inserted_at_top` also clears every cached page, not only the
//! ones that moved — and `GtkListView`'s own anchor-preservation does not
//! reliably survive that: measured landing anywhere from "sitting at the
//! very top, new mail shows up 220px below the viewport" to "scrolled well
//! away, the whole view snaps back to zero" depending on where the viewport
//! already was. The fix (see `MessageListView`'s `items_changed` handler)
//! captures whatever the scroll offset already was the instant the model
//! changed, and reasserts exactly that value for a few frames until GTK
//! stops fighting it — which reveals new mail at the top (the captured
//! value is 0) and leaves everyone else exactly where they were (the
//! captured value is whatever they had scrolled to), from the one
//! correction. This file proves both halves.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;

/// A mailbox whose length can grow mid-test, the way new mail arriving does
/// — with stable ids, so a message already on screen is still the same
/// message afterward and not a different one that happens to share its
/// position. `original` ids run 1..=original, oldest-numbered at the
/// bottom; an arrival's ids run upward from there, newest (highest id)
/// first, and every original row's *position* shifts down by however many
/// have arrived so far.
struct Store {
    original: u32,
    arrived: Cell<u32>,
}

impl Store {
    fn new(original: u32) -> Rc<Self> {
        Rc::new(Store {
            original,
            arrived: Cell::new(0),
        })
    }

    /// `n` messages landed at the top.
    fn arrived(&self, n: u32) {
        self.arrived.set(self.arrived.get() + n);
    }

    fn total(&self) -> u32 {
        self.original + self.arrived.get()
    }
}

impl MailboxSource for Store {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let account = AccountId::new(ACCOUNT);
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: self.total(),
            unread: self.total(),
            flagged: 0,
            snoozed: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

impl MessageSource for Store {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = self.total();
        let arrived = self.arrived.get();
        let original = self.original;
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| {
                    let id = if position < arrived {
                        (original + arrived - position) as i64
                    } else {
                        (position - arrived) as i64 + 1
                    };
                    Row {
                        id: MessageId::new(id),
                        thread: None,
                        from: None,
                        subject: Some(format!("message {id}")),
                        preview: None,
                        received_at: Utc.timestamp_opt(1_700_000_000 - id, 0).unwrap(),
                        seen: false,
                        flagged: false,
                        answered: false,
                        draft: false,
                        has_attachments: false,
                        thread_count: 1,
                        participants: Vec::new(),
                    }
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

pub fn new_mail_reveals_itself_at_the_top_and_nowhere_else() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    // Enough rows that the list genuinely overflows the viewport -- the
    // whole point is a scroll position, and there is nothing to scroll
    // through fifteen rows.
    let store = Store::new(300);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    let list = window.list();
    pump_until(|| list.model().n_items() == 300);
    assert_eq!(list.model().n_items(), 300, "the folder as it stands");

    // ── at the top: new mail is revealed, nothing else moves ──────────────
    assert_eq!(
        list.scroll_offset(),
        0.0,
        "opening a folder starts at the top"
    );
    let cursor_before = list.cursor_id().expect("the cursor is on a row");

    store.arrived(3);
    feeds.apply(&Event::NewMail {
        account: AccountId::new(ACCOUNT),
        mailbox: MailboxId::new(INBOX),
        messages: vec![
            MessageId::new(301),
            MessageId::new(302),
            MessageId::new(303),
        ],
    });
    pump_until(|| list.model().n_items() == 303);
    // The anchor-preserving shift GTK makes on its own happens during
    // layout, on the frame clock -- a plain event-loop drain does not
    // advance it, so this is the one wait in the file that has to.
    settle_frames(&window);

    assert_eq!(
        list.scroll_offset(),
        0.0,
        "sitting at the top, new mail must stay revealed rather than land \
         above the viewport"
    );
    assert_eq!(
        list.cursor_id(),
        Some(cursor_before),
        "the cursor stays on the same message -- inserted_at_top moves it \
         down with its row, it does not move what is being read"
    );

    // ── scrolled away: nothing moves, exactly as #72 already requires ─────
    for _ in 0..5 {
        list.next_row();
    }
    pump_until(|| list.cursor_id() != Some(cursor_before));
    let scrolled_cursor = list.cursor_id().expect("the cursor moved");

    list.set_scroll_offset(2_000.0);
    pump();
    let offset_before = list.scroll_offset();
    assert!(
        offset_before > 0.0,
        "the offset should have taken, or this proves nothing: {offset_before}"
    );

    store.arrived(2);
    feeds.apply(&Event::NewMail {
        account: AccountId::new(ACCOUNT),
        mailbox: MailboxId::new(INBOX),
        messages: vec![MessageId::new(304), MessageId::new(305)],
    });
    pump_until(|| list.model().n_items() == 305);
    settle_frames(&window);

    assert_eq!(
        list.scroll_offset(),
        offset_before,
        "scrolled away, new mail must not move the viewport"
    );
    assert_eq!(
        list.cursor_id(),
        Some(scrolled_cursor),
        "and must not move the cursor either"
    );

    window.destroy();
}

/// Run the main loop until it has nothing left to do.
///
/// The feeds answer on the thread-default main context, so nothing a page
/// request set in motion has happened until this returns.
fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.pending() {
            context.iteration(false);
        }
    }
}

/// Pump until `done`, or give up after a deadline.
///
/// See `gtk_list_reload.rs`'s own copy of this helper for why waiting on
/// the condition, rather than a fixed number of passes, is what keeps this
/// strict on a loaded machine instead of merely slow.
fn pump_until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        pump();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done(), "timed out waiting for the list to catch up");
}

/// Let ten real frames pass on `window`'s frame clock.
///
/// Only the anchor-shift correction under test needs this: everything else
/// here is plain model/event-loop state that `pump`/`pump_until` already
/// reach without waiting on a compositor frame at all.
fn settle_frames(window: &Window) {
    let left = Rc::new(Cell::new(10u32));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_secs(20);
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}
