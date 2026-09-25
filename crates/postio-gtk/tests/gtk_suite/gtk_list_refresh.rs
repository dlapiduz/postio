//! A change the list cannot describe is a refresh in place, not a reload
//! (maintainer, 2026-09-25: "full reloads make the app feel slow").
//!
//! `MessageListChanged` -- every sync batch, every resync with a flag in it,
//! the snooze timer -- and new mail, and a removal in a view that cannot
//! take a row out by itself, all used to drop every page and announce every
//! position as replaced. Every row on screen flashed to a skeleton and was
//! built again. These drive the real feed through a real window and count
//! what the list told its view: nothing, when nothing moved; one row in or
//! out at its position, when one did; and never a placeholder on screen.
//!
//! Skips without a display. Nothing here touches the network.

use crate::pump;
use std::cell::RefCell;
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

/// A folder whose order a test can move, newest first.
struct Folder {
    order: RefCell<Vec<i64>>,
}

impl Folder {
    fn new(count: i64) -> Rc<Self> {
        Rc::new(Folder {
            order: RefCell::new((1..=count).collect()),
        })
    }
}

impl MailboxSource for Folder {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let mut inbox = Mailbox::new(AccountId::new(ACCOUNT), "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        let total = self.order.borrow().len() as u32;
        inbox.counts = MailboxCounts {
            total,
            unread: 0,
            flagged: 0,
            snoozed: 0,
            attention: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

/// The second folder: a different mailbox, a different set of mail.
const ARCHIVE: i64 = 2;

fn row(id: i64) -> Row {
    Row {
        id: MessageId::new(id),
        thread: None,
        from: None,
        subject: Some(format!("message {id}")),
        preview: None,
        received_at: Utc.timestamp_opt(1_700_000_000 - id, 0).unwrap(),
        seen: true,
        flagged: false,
        answered: false,
        send_state: None,
        send_at: None,
        has_attachments: false,
        thread_count: 1,
        participants: Vec::new(),
    }
}

impl postio_gtk::feed::ResultSource for Folder {
    fn rows(&self, ids: Vec<MessageId>) -> postio_gtk::feed::RowsFuture {
        Box::pin(async move { Ok(ids.into_iter().map(|id| row(id.get())).collect()) })
    }
}

impl MessageSource for Folder {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let order = if request.scope == postio_model::ListScope::Mailbox(MailboxId::new(ARCHIVE)) {
            (5000..5080).collect()
        } else {
            self.order.borrow().clone()
        };
        Box::pin(async move {
            let total = order.len() as u32;
            let start = (request.offset as usize).min(order.len());
            let end = ((request.offset + request.limit) as usize).min(order.len());
            let rows = order[start..end].iter().map(|id| row(*id)).collect();
            Ok(Page { total, rows })
        })
    }
}

fn changed() -> Event {
    Event::MessageListChanged {
        account: AccountId::new(ACCOUNT),
        mailbox: MailboxId::new(INBOX),
    }
}

fn pump_until(done: impl Fn() -> bool) -> bool {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        pump();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    done()
}

/// Let whatever an event set in motion finish: a short quiet spell.
fn settle() {
    for _ in 0..20 {
        pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A window over `folder`, loaded, with the cursor a few rows down.
fn open(folder: &Rc<Folder>) -> Option<(Window, postio_gtk::feed::Feeds)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = Window::default();
    window.set_default_size(1200, 800);
    window.present();
    pump();
    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        folder.clone(),
        folder.clone(),
    );
    let list = window.list();
    let total = folder.order.borrow().len() as u32;
    assert!(
        pump_until(|| list.model().n_items() == total && every_row_is_mail(&window)),
        "the folder never loaded"
    );
    for _ in 0..6 {
        list.next_row();
    }
    settle();
    Some((window, feeds))
}

/// Whether every row widget on screen is drawing mail rather than a
/// skeleton -- what a person sees, rather than what the model holds.
fn every_row_is_mail(window: &Window) -> bool {
    let rows = std::cell::Cell::new(0);
    let mail = std::cell::Cell::new(0);
    window.list().each_row(|row| {
        rows.set(rows.get() + 1);
        if row.row().is_some() {
            mail.set(mail.get() + 1);
        }
    });
    rows.get() > 0 && rows.get() == mail.get()
}

/// How many pages the view holds row widgets in. GTK keeps a band of rows
/// realised around the viewport, not only the visible ones, and every one of
/// them has to be right when it scrolls into view -- so this, and not one
/// screenful, is what a refresh may re-read.
fn pages_held(window: &Window) -> u64 {
    let pages = RefCell::new(std::collections::BTreeSet::new());
    window.list().each_row(|row| {
        pages
            .borrow_mut()
            .insert(row.index() / postio_gtk::list::PAGE_SIZE);
    });
    pages.into_inner().len() as u64
}

/// Every `items_changed` the list emits from now on.
fn record(window: &Window) -> Rc<RefCell<Vec<(u32, u32, u32)>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    window.list().model().connect_items_changed({
        let seen = seen.clone();
        move |_, position, removed, added| seen.borrow_mut().push((position, removed, added))
    });
    seen
}

pub fn a_resync_that_moved_nothing_tells_the_view_nothing() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    let cursor = window.list().cursor_id();
    let seen = record(&window);
    let pages = postio_ui::test_support::pages_requested();
    let held = pages_held(&window);

    feeds.apply(&changed());
    assert!(
        every_row_is_mail(&window),
        "a resync put skeletons on screen before it had read anything"
    );
    settle();

    assert!(
        seen.borrow().is_empty(),
        "nothing moved, and the list told its view {:?}",
        seen.borrow()
    );
    let read = postio_ui::test_support::pages_requested() - pages;
    assert!(
        read <= held,
        "a resync read {read} pages, more than the {held} the view holds rows in"
    );
    assert!(every_row_is_mail(&window));
    assert_eq!(window.list().cursor_id(), cursor);
}

pub fn mail_landing_on_top_arrives_with_its_contents() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    let cursor = window.list().cursor_id();
    let seen = record(&window);
    let pages = postio_ui::test_support::pages_requested();
    let held = pages_held(&window);

    folder.order.borrow_mut().insert(0, 999);
    feeds.apply(&Event::NewMail {
        account: AccountId::new(ACCOUNT),
        mailbox: MailboxId::new(INBOX),
        messages: vec![MessageId::new(999)],
    });
    assert!(
        every_row_is_mail(&window),
        "new mail put a skeleton on screen before it had been read"
    );
    let model = window.list().model();
    assert!(
        pump_until(|| {
            assert!(every_row_is_mail(&window), "a skeleton was on screen");
            model.peek(0) == Some(MessageId::new(999))
        }),
        "the new mail never arrived"
    );
    settle();

    assert_eq!(model.n_items(), 121);
    let read = postio_ui::test_support::pages_requested() - pages;
    assert!(
        read <= held,
        "one new message read {read} pages, more than the {held} the view holds rows in"
    );
    assert!(
        seen.borrow().contains(&(0, 0, 1)),
        "the new row was not inserted at the top: {:?}",
        seen.borrow()
    );
    assert!(
        seen.borrow()
            .iter()
            .all(|(_, removed, added)| *removed <= 1 && *added <= 1),
        "a row landing on top replaced a stretch of the list: {:?}",
        seen.borrow()
    );
    assert_eq!(
        window.list().cursor_id(),
        cursor,
        "the cursor left its message"
    );
}

pub fn a_message_leaving_is_one_row_taken_out_where_it_stood() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    let cursor = window.list().cursor_id();
    let seen = record(&window);

    folder.order.borrow_mut().retain(|id| *id != 3);
    feeds.apply(&changed());
    let model = window.list().model();
    assert!(
        pump_until(|| model.n_items() == 119),
        "the removal never arrived"
    );
    settle();

    assert!(
        seen.borrow().contains(&(2, 1, 0)),
        "the row was not taken out where it stood: {:?}",
        seen.borrow()
    );
    assert!(
        seen.borrow()
            .iter()
            .all(|(_, removed, added)| *removed <= 1 && *added <= 1),
        "one message leaving replaced a stretch of the list: {:?}",
        seen.borrow()
    );
    assert_eq!(model.peek(2), Some(MessageId::new(4)));
    assert!(every_row_is_mail(&window));
    assert_eq!(
        window.list().cursor_id(),
        cursor,
        "the cursor left its message"
    );
}

pub fn a_conversation_moving_to_the_top_keeps_the_cursor_on_it() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    let reading = window.list().cursor_id().expect("the cursor is on a row");

    {
        let mut order = folder.order.borrow_mut();
        order.retain(|id| *id != reading.get());
        order.insert(0, reading.get());
    }
    feeds.apply(&changed());
    let model = window.list().model();
    assert!(
        pump_until(|| model.peek(0) == Some(reading)),
        "the move never arrived"
    );
    settle();
    assert_eq!(
        window.list().cursor_id(),
        Some(reading),
        "the conversation moved and the cursor stayed on its old position"
    );
}

pub fn switching_folders_keeps_the_rows_until_the_new_ones_land() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    let seen = record(&window);

    feeds
        .messages
        .open(postio_model::ListScope::Mailbox(MailboxId::new(ARCHIVE)));
    assert!(
        every_row_is_mail(&window),
        "switching folders put skeletons on screen before the folder was read"
    );
    assert!(
        !matches!(
            window.list_state().state(),
            Some(postio_ui::list_state::State::InboxZero { .. })
        ),
        "the list said the folder was empty before it had been read"
    );
    let model = window.list().model();
    assert!(
        pump_until(|| model.peek(0) == Some(MessageId::new(5000))),
        "the other folder never arrived"
    );
    assert!(
        every_row_is_mail(&window),
        "the new folder's rows are skeletons"
    );
    assert_eq!(model.n_items(), 80);
    assert_eq!(
        seen.borrow().as_slice(),
        &[(0, 120, 50), (50, 0, 30)],
        "a folder switch is one change-over with its first page in hand, \
         and the rest of the count arriving as an insertion at the end"
    );
}

pub fn leaving_a_search_puts_the_folder_back_without_reading_it() {
    let folder = Folder::new(120);
    let Some((window, feeds)) = open(&folder) else {
        return;
    };
    feeds.messages.set_result_source(folder.clone());
    let model = window.list().model();
    feeds.messages.show_results(vec![
        MessageId::new(40),
        MessageId::new(7),
        MessageId::new(90),
    ]);
    assert!(
        pump_until(|| model.n_items() == 3 && every_row_is_mail(&window)),
        "the results never arrived"
    );

    let seen = record(&window);
    assert!(feeds.messages.close_results());
    assert_eq!(
        model.n_items(),
        120,
        "the folder is back at its length at once"
    );
    assert!(
        every_row_is_mail(&window),
        "leaving a search put skeletons on screen while the folder was read again"
    );
    let changes = seen.borrow().clone();
    assert!(
        matches!(changes.first(), Some((0, 3, held)) if *held > 0),
        "leaving a search is one change-over back to the folder's held rows: {changes:?}"
    );
    assert!(
        changes.len() <= 2,
        "and at most the rest of its count after them: {changes:?}"
    );
    settle();
    assert!(every_row_is_mail(&window));
    assert_eq!(model.peek(0), Some(MessageId::new(1)));
}

pub fn an_empty_folder_does_not_speak_for_the_next_one_while_it_loads() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let folder = Folder::new(0);
    let window = Window::default();
    window.present();
    pump();
    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        folder.clone(),
        folder.clone(),
    );
    feeds.folders.apply(&Event::ConnectionChanged {
        account: AccountId::new(ACCOUNT),
        state: postio_core::ConnectionState::Online,
    });
    let empty = || {
        matches!(
            window.list_state().state(),
            Some(postio_ui::list_state::State::InboxZero { .. })
        )
    };
    let said = pump_until(empty);
    assert!(
        said,
        "an empty inbox should say so: {:?}",
        window.list_state().state()
    );

    feeds
        .messages
        .open(postio_model::ListScope::Mailbox(MailboxId::new(ARCHIVE)));
    assert!(
        !empty(),
        "the list called the next folder empty before it had read a row of it"
    );
    let model = window.list().model();
    assert!(pump_until(|| model.n_items() == 80));
    assert!(!empty());
}
