//! Marking a message read must not refresh the list.
//!
//! Reading a message is the most ordinary thing that happens in a mail client,
//! and it changes one flag on one row. The engine announces it as
//! `MessagesChanged`, and for an ordinary mailbox the feed answers by
//! refetching only the pages holding those messages -- `ListWindow::deliver`
//! then replaces the data *inside* the rows that are already resident rather
//! than making new ones, precisely so that nothing holding a row has to
//! rediscover it.
//!
//! What undid that was the announcement afterwards: a page delivery emitted
//! `items_changed(start, 50, 50)`, which tells `GtkListView` that fifty items
//! were removed and fifty different ones inserted. It answers the only way it
//! can -- by dropping the widgets for that range and building them again --
//! and the user sees the whole visible list blink for a flag they set on one
//! row. Reported by the maintainer, 2026-09-07.
//!
//! Counted, not timed: an emission and a row widget are the same numbers on
//! every machine. Skips without a display. Nothing here touches the network.

use crate::pump;
use std::cell::Cell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
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
const TOTAL: u32 = 120;

/// A mailbox where exactly one message can become read.
struct Mailbox120 {
    read: Cell<Option<i64>>,
}

impl Mailbox120 {
    fn new() -> Rc<Self> {
        Rc::new(Mailbox120 {
            read: Cell::new(None),
        })
    }

    /// The user opened a message; the store now says it is seen.
    fn mark_read(&self, id: MessageId) {
        self.read.set(Some(id.get()));
    }
}

impl MailboxSource for Mailbox120 {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let account = AccountId::new(ACCOUNT);
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: TOTAL,
            unread: TOTAL - u32::from(self.read.get().is_some()),
            flagged: 0,
            snoozed: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

impl MessageSource for Mailbox120 {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let read = self.read.get();
        Box::pin(async move {
            let end = (request.offset + request.limit).min(TOTAL);
            let rows = (request.offset..end)
                .map(|position| {
                    let id = position as i64 + 1;
                    Row {
                        id: MessageId::new(id),
                        thread: None,
                        from: None,
                        subject: Some(format!("message {position}")),
                        preview: None,
                        received_at: Utc
                            .timestamp_opt(1_700_000_000 - position as i64, 0)
                            .unwrap(),
                        seen: read == Some(id),
                        flagged: false,
                        answered: false,
                        draft: false,
                        has_attachments: false,
                        thread_count: 1,
                        participants: Vec::new(),
                    }
                })
                .collect();
            Ok(Page { total: TOTAL, rows })
        })
    }
}

/// Whether the model says the message at `position` has been read.
fn seen_at(list: &postio_gtk::list_view::MessageListView, position: u32) -> bool {
    list.model()
        .item(position)
        .and_downcast::<postio_gtk::list::MessageRow>()
        .and_then(|row| row.row())
        .is_some_and(|row| row.seen)
}

pub fn marking_a_message_read_does_not_rebuild_the_list() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Mailbox120::new();
    let window = Window::default();
    window.set_default_size(1280, 800);
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
    pump_until(|| list.model().n_items() == TOTAL);
    assert_eq!(list.model().n_items(), TOTAL, "the folder as it stands");
    assert!(!seen_at(&list, 2), "the message starts unread");

    // Settle first: the count below must measure the flag, not the fill.
    for _ in 0..8 {
        pump();
    }
    let emissions = postio_gtk::list::emissions();
    let widgets = postio_gtk::row::rows_built();

    // The user opens the third message.
    let opened = MessageId::new(3);
    store.mark_read(opened);
    feeds.apply(&Event::MessagesChanged {
        account: postio_model::AccountId::new(ACCOUNT),
        messages: vec![opened],
    });
    pump_until(|| seen_at(&list, 2));

    assert!(
        seen_at(&list, 2),
        "the row never picked up the read flag, so nothing below means anything"
    );
    assert!(
        !seen_at(&list, 1) && !seen_at(&list, 3),
        "reading one message marked its neighbours too"
    );
    assert_eq!(
        postio_gtk::list::emissions() - emissions,
        0,
        "reading one message told the view that a page of rows was replaced, \
         which is what makes the whole visible list blink"
    );
    assert_eq!(
        postio_gtk::row::rows_built() - widgets,
        0,
        "reading one message rebuilt row widgets; the rows were already there \
         and only their contents changed"
    );
}

/// Filling a folder must announce structure, not content.
///
/// Opening a folder tells the model two structural things: the previous scope
/// is gone, and the new one is this long. Everything after that is a page of
/// rows landing on positions that already exist and are showing placeholders.
///
/// Announcing those as `items_changed(start, 50, 50)` costs a widget per row
/// in range -- measured exactly, on a list whose viewport holds ten: an insert
/// of 799 built 205 widgets, and then every page delivery that landed inside
/// those 205 rebuilt 50 more. Half the widgets a folder switch built were
/// rebuilding rows that had just been built (#1216).
///
/// So the placeholders are the rows: `item` hands out one object per position
/// and keeps handing out that same object, a page delivery fills it in, and it
/// says so for itself. Counted, not timed. Skips without a display.
pub fn filling_a_folder_announces_structure_and_not_every_page() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Mailbox120::new();
    let window = Window::default();
    window.set_default_size(1280, 800);
    window.present();
    pump();

    let emissions = postio_gtk::list::emissions();
    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == TOTAL);
    for _ in 0..8 {
        pump();
    }

    assert_eq!(list.model().n_items(), TOTAL, "the folder as it stands");
    assert!(
        list.model()
            .item(0)
            .and_downcast::<postio_gtk::list::MessageRow>()
            .is_some_and(|row| row.is_loaded()),
        "the rows never arrived, so the count below proves nothing"
    );
    assert!(
        !seen_at(&list, 0),
        "the rows arrived but carry the wrong contents"
    );

    // Two: the scope changed, and it is this long. A page landing on
    // positions that already exist is not a third.
    let structural = postio_gtk::list::emissions() - emissions;
    assert!(
        structural <= 2,
        "filling a folder took {structural} `items_changed`; a page of rows          landing on positions that already exist is not a structural change,          and each one costs a widget per row in range"
    );
    let _ = feeds;
}

/// Pump until `done`, the same local helper `gtk_list_reload` keeps.
fn pump_until(done: impl Fn() -> bool) {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        pump();
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
