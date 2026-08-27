//! Feeding the message list, without a database, a display or a runtime.
//!
//! [`MessageSource`] is one method, which is what makes the whole of this
//! checkable against a table: a fake source records what it was asked for and
//! the test decides when — and whether — to answer. Nothing here touches the
//! network.
//!
//! One test function, for the reason `gtk_style.rs` gives, and for one more
//! of its own: the replies are awaited on the thread-default main context,
//! and the test harness runs test functions on threads of its own. Two of
//! these running at once would have one thread driving the other's futures,
//! which glib refuses — loudly, and only sometimes.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_gtk::feed::{Feed, FeedScope, MessageSource, Page, PageFuture, PageRequest};
use postio_gtk::list::{MessageList, MessageRow, PAGE_SIZE, Row};
use postio_model::ids::{MailboxId, MessageId};

/// Two mailboxes, so "the reply is for the folder you left" is testable.
const INBOX: i64 = 1;
const ARCHIVE: i64 = 2;

/// A source that answers from a count, and remembers every question.
///
/// Answers are queued rather than immediate: `fetch` returns a future the
/// test resolves by pumping the main context, which is how a real answer
/// arrives too.
#[derive(Default)]
struct Fake {
    asked: RefCell<Vec<PageRequest>>,
    totals: RefCell<Vec<(i64, u32)>>,
    /// Mailboxes whose reads fail, and why.
    broken: RefCell<Vec<(i64, String)>>,
}

/// The folder a request names. These tests open folders, not smart folders —
/// `gtk_flagged.rs` is where the query scope is exercised.
fn scope_mailbox(request: &PageRequest) -> MailboxId {
    request
        .scope
        .mailbox()
        .expect("these tests open folders, not queries")
}

impl Fake {
    fn new() -> Rc<Self> {
        Rc::new(Fake::default())
    }

    fn holding(self: &Rc<Self>, mailbox: i64, total: u32) -> Rc<Self> {
        self.totals.borrow_mut().push((mailbox, total));
        self.clone()
    }

    fn breaking(self: &Rc<Self>, mailbox: i64, reason: &str) -> Rc<Self> {
        self.broken.borrow_mut().push((mailbox, reason.to_string()));
        self.clone()
    }

    fn drain(&self) -> Vec<PageRequest> {
        self.asked.borrow_mut().drain(..).collect()
    }

    fn total_of(&self, mailbox: MailboxId) -> u32 {
        self.totals
            .borrow()
            .iter()
            .find(|(id, _)| MailboxId::new(*id) == mailbox)
            .map(|(_, total)| *total)
            .unwrap_or(0)
    }
}

impl MessageSource for Fake {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        self.asked.borrow_mut().push(request);
        let broken = self
            .broken
            .borrow()
            .iter()
            .find(|(id, _)| MailboxId::new(*id) == scope_mailbox(&request))
            .map(|(_, reason)| reason.clone());
        let total = self.total_of(scope_mailbox(&request));
        let mailbox = scope_mailbox(&request);
        Box::pin(async move {
            if let Some(reason) = broken {
                return Err(reason);
            }
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| row(mailbox, position))
                .collect();
            Ok(Page { total, rows })
        })
    }
}

/// A row whose id encodes the mailbox and the position it came from, so a
/// test can say whose mail it is looking at.
fn row(mailbox: MailboxId, position: u32) -> Row {
    Row {
        id: MessageId::new(mailbox.get() * 1_000_000 + position as i64 + 1),
        thread: None,
        from: None,
        subject: Some(format!("mailbox {} message {position}", mailbox.get())),
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
    }
}

/// Let every future that is ready to finish, finish.
fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

fn loaded(list: &MessageList, position: u32) -> Option<Row> {
    list.item(position)
        .and_downcast::<MessageRow>()
        .and_then(|item| item.row())
}

pub fn the_message_list_is_fed_from_the_runtime() {
    // ── opening a mailbox shows its rows and only asks for what it draws ────
    let source = Fake::new().holding(INBOX, 100_000);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());

    assert_eq!(list.n_items(), 0, "nothing is showing before a mailbox is");

    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    assert_eq!(feed.mailbox(), Some(MailboxId::new(INBOX)));
    assert_eq!(
        source.drain(),
        [PageRequest {
            scope: FeedScope::Mailbox(MailboxId::new(INBOX)),
            page: 0,
            offset: 0,
            limit: PAGE_SIZE,
        }],
        "opening asks for the first page and nothing else"
    );
    // The count is not known until the read comes back, and the list says so
    // rather than guessing.
    assert_eq!(list.n_items(), 0);

    settle();
    assert_eq!(
        list.n_items(),
        100_000,
        "the page carried the count with it"
    );
    assert_eq!(
        loaded(&list, 0).and_then(|row| row.subject),
        Some("mailbox 1 message 0".to_string())
    );
    // The view's own first look at position 0 finds it already cached, so
    // opening a folder costs exactly one read.
    assert!(
        source.drain().is_empty(),
        "the first page was fetched twice"
    );

    // A hundred thousand messages, and only what was looked at is resident.
    settle();
    assert!(
        list.resident_rows() <= PAGE_SIZE as usize * 3,
        "{} rows resident after opening a folder",
        list.resident_rows()
    );

    // ── a reply for the folder you left never lands in the one you opened ────
    // The normal case, not an edge case: picking two folders quickly means
    // the first answer arrives after the second is on screen.
    let source = Fake::new().holding(INBOX, 500).holding(ARCHIVE, 7);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());

    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    feed.open(FeedScope::Mailbox(MailboxId::new(ARCHIVE)));
    settle();

    assert_eq!(
        list.n_items(),
        7,
        "the list is showing the folder it was told to"
    );
    assert_eq!(
        loaded(&list, 0).and_then(|row| row.subject),
        Some("mailbox 2 message 0".to_string()),
        "the abandoned folder's mail landed in the open one"
    );

    // ── new mail arrives without the list losing its place ──────────────────
    let source = Fake::new().holding(INBOX, 200);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());
    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    settle();

    // Hold the object at a position the way a selection model does.
    let anchor = list.item(3).and_downcast::<MessageRow>().unwrap();
    let anchored = anchor.id();

    // The *shape* of the change, not merely its outcome. Issue #72: the
    // rows end up correct either way, so an assertion about content passes
    // whether this is a two-row insertion or a reset of the whole list —
    // and a reset is what discards every visible row widget, re-reads every
    // page, and makes the list flicker while the user is reading.
    let changes = Rc::new(RefCell::new(Vec::new()));
    list.connect_items_changed({
        let changes = changes.clone();
        move |_, position, removed, added| changes.borrow_mut().push((position, removed, added))
    });

    feed.apply(&Event::NewMail {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
        messages: vec![MessageId::new(9_001), MessageId::new(9_002)],
    });

    assert_eq!(
        *changes.borrow(),
        [(0, 0, 2)],
        "new mail has to arrive as an insertion at the top. A reset here \
         reads as `(0, 200, 202)` and costs every row widget on screen"
    );
    assert_eq!(list.n_items(), 202, "two arrived");
    assert_eq!(
        anchor.id(),
        anchored,
        "the row an anchor is holding was replaced rather than moved"
    );

    // Nothing about another mailbox moves this one.
    feed.apply(&Event::NewMail {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(ARCHIVE),
        messages: vec![MessageId::new(9_003)],
    });
    assert_eq!(
        list.n_items(),
        202,
        "another folder's mail changed this one"
    );

    // ── a changed message costs its page and keeps its identity ─────────────
    let source = Fake::new().holding(INBOX, 500);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());
    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    settle();

    // Reach into a second page so there is more than one to be wrong about.
    list.item(PAGE_SIZE * 2);
    settle();
    source.drain();

    let item = list.item(1).and_downcast::<MessageRow>().unwrap();
    let changed = item.id().unwrap();

    feed.apply(&Event::MessagesChanged {
        account: postio_model::AccountId::new(1),
        messages: vec![changed, MessageId::new(404_404)],
    });

    let asked: Vec<u32> = source.drain().iter().map(|request| request.page).collect();
    assert_eq!(
        asked,
        [0],
        "a changed message should cost its own page and no other"
    );

    settle();
    assert_eq!(
        list.item(1).and_downcast::<MessageRow>().unwrap(),
        item,
        "the row was rebuilt rather than updated in place"
    );

    // ── an emptied mailbox stops showing the mail it used to have ───────────
    let source = Fake::new().holding(INBOX, 120);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());
    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    settle();
    assert_eq!(list.n_items(), 120);

    // Everything archived at once. A list that only refetched when asked
    // would keep drawing rows nobody can open.
    source.totals.borrow_mut().clear();
    source.totals.borrow_mut().push((INBOX, 0));
    feed.apply(&Event::MessagesRemoved {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
        messages: (1..=120).map(MessageId::new).collect(),
    });
    settle();

    assert_eq!(list.n_items(), 0);

    // ── a read that fails says why rather than showing nothing ──────────────
    let source = Fake::new().breaking(INBOX, "the database is locked");
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());

    let reported = Rc::new(RefCell::new(Vec::new()));
    feed.connect_error({
        let reported = reported.clone();
        move |reason| reported.borrow_mut().push(reason)
    });

    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    settle();

    assert_eq!(
        reported.borrow().as_slice(),
        ["the database is locked".to_string()],
        "a failed read was swallowed"
    );
}
