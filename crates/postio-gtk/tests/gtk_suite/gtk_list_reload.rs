//! A batch landing mid-sync must not move what the user is holding.
//!
//! `postio-qhz.7`'s fourth acceptance criterion. A sync pass announces itself
//! as its batches commit, and each announcement costs the list a reload — so
//! the reload is not a rare event during a first sync, it is the steady state
//! for minutes. If a reload moved the cursor or dropped the marked messages,
//! `x x x a` would archive the wrong mail, and it would do it only on a real
//! account with a real sync running, which is the hardest place to notice.
//!
//! Two separate things have to survive, and they survive for different
//! reasons. The *selection* is `SelectionState`, a set of `MessageId`s, so it
//! is not addressed by position at all. The *cursor* is a `GtkSingleSelection`
//! over the model, which is addressed by position — it survives because
//! `MessageList::invalidate` re-emits `items_changed` over the same length
//! rather than resetting the model, and because the page that comes back
//! carries the same ids in the same order.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;

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

/// A mailbox a sync is still filling.
///
/// Rows are oldest-last and their ids are stable, which is what an initial
/// sync actually does: it walks the newest UID down, so a batch lands at the
/// *end* of the list and the rows already on screen keep their positions.
struct Filling {
    total: Cell<u32>,
}

impl Filling {
    fn new(total: u32) -> Rc<Self> {
        Rc::new(Filling {
            total: Cell::new(total),
        })
    }

    /// A batch committed: the folder is longer than it was.
    fn batch_committed(&self, rows: u32) {
        self.total.set(self.total.get() + rows);
    }
}

impl MailboxSource for Filling {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let account = AccountId::new(ACCOUNT);
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: self.total.get(),
            unread: self.total.get(),
            flagged: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

impl MessageSource for Filling {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = self.total.get();
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: MessageId::new(position as i64 + 1),
                    thread: None,
                    from: None,
                    subject: Some(format!("message {position}")),
                    preview: None,
                    received_at: Utc
                        .timestamp_opt(1_700_000_000 - position as i64, 0)
                        .unwrap(),
                    seen: false,
                    flagged: false,
                    answered: false,
                    draft: false,
                    has_attachments: false,
                    thread_count: 1,
                    participants: Vec::new(),
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

pub fn a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Filling::new(120);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == 120);
    assert_eq!(list.model().n_items(), 120, "the folder as it stands");

    // Where the user is: a few rows down, with three messages marked.
    for _ in 0..6 {
        list.next_row();
    }
    pump();
    let cursor = list.cursor_id().expect("the cursor is on a row");
    let marked = [MessageId::new(2), MessageId::new(4), MessageId::new(9)];
    list.selection().extend_over(marked);
    let selection = list.selection().selection();

    // A batch commits. The engine coalesces these and announces the mailbox
    // moved; it does not say how, because it does not know.
    store.batch_committed(200);
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.model().n_items() == 320);

    assert_eq!(
        list.model().n_items(),
        320,
        "the batch has to actually arrive, or this test proves nothing"
    );
    assert_eq!(
        list.cursor_id(),
        Some(cursor),
        "the cursor moved to a different message when a batch landed"
    );
    assert_eq!(
        list.selection().selection(),
        selection,
        "marked messages were lost when a batch landed; the next verb would \
         have hit the wrong mail"
    );
    for message in marked {
        assert!(
            list.selection().contains(message),
            "message {} is no longer marked",
            message.get()
        );
    }

    // And again, several times over, because a real sync does this for
    // minutes rather than once.
    for batch in 0..5 {
        store.batch_committed(200);
        feeds.apply(&Event::MessageListChanged {
            account: postio_model::AccountId::new(1),
            mailbox: MailboxId::new(INBOX),
        });
        let expected = 520 + batch * 200;
        pump_until(|| list.model().n_items() == expected);
    }
    assert_eq!(list.model().n_items(), 1_320);
    assert_eq!(list.cursor_id(), Some(cursor), "drifted over six batches");
    assert_eq!(list.selection().selection(), selection);
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
/// `pump` spins two hundred times over whatever is *currently* pending, which
/// is a bet that the work will have been queued by the time it looks. With
/// several sessions building, a task can be spawned and not yet scheduled
/// when `pending()` reports false, so `pump` returns before the page it set
/// in motion has landed and the count read after it is one batch stale
/// (`postio-mdu1`).
///
/// Waiting on the condition keeps the assertion exactly as strict — a batch
/// that genuinely never arrives still fails, after the deadline — while a
/// loaded machine only makes it wait longer.
fn pump_until(done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        pump();
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
