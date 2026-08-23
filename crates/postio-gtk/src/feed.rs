//! Where the message list's rows actually come from.
//!
//! [`crate::list`] built the windowed model and left a hole in it: a
//! [`PageSource`] that says *which page it needs* and waits. This module
//! fills that hole, and drives the model from the runtime's event stream.
//!
//! # Why there is a trait here and not a repository call
//!
//! `postio-gtk` must not depend on `rusqlite` — CI enforces it — so the
//! frontend cannot read `postio-storage`'s message repository itself. The
//! page fetch has to cross to the runtime and come back, and the runtime is
//! on tokio worker threads while every widget here is main-thread only.
//!
//! [`MessageSource`] is that crossing, expressed as the only thing the list
//! actually needs: *give me these rows, eventually*. The future it returns is
//! awaited with `glib::spawn_future_local`, so the answer lands on the main
//! thread and nothing here ever blocks on the network or on SQLite. What
//! implements it is a `postio-core` concern — see the module's own note on
//! what is still missing.
//!
//! # What it costs to be wrong about the mailbox
//!
//! Switching folders while a page is in flight is the normal case, not an
//! edge case: the answer for the old folder arrives after the new one is on
//! screen. Every request carries the generation it was made in, and a reply
//! from an older generation is dropped. Without that, picking a folder
//! quickly twice fills the second one with the first one's mail.
//!
//! # Driving the model from events
//!
//! | Event | What the list does |
//! |---|---|
//! | [`Event::NewMail`] | [`MessageList::inserted_at_top`] — the selection and the scroll anchor move down with their row |
//! | [`Event::MessagesChanged`] | refetch only the resident pages holding them, so the rows keep their `GObject` and nothing loses its place |
//! | [`Event::MessagesRemoved`] | refetch; the row count moved |
//! | [`Event::MessageListChanged`] | refetch; the order itself moved |
//!
//! Anything about another mailbox is ignored outright — the list shows one
//! mailbox, and a folder nobody is looking at costs nothing to be wrong
//! about until it is opened.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_model::ids::MailboxId;

use crate::list::{MessageList, PAGE_SIZE, PageSource, Row};

/// One page of a mailbox, as the runtime answered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// How many rows the mailbox has in total, as of this read.
    ///
    /// Carried with every page rather than asked for separately: the count
    /// and the rows have to come from one read of the database, or the list
    /// can be told about a total that no page will ever fill.
    pub total: u32,
    /// The rows themselves, in list order.
    ///
    /// [`Row::thread_count`] is expected to be real here — the badge in the
    /// canvas is a count of the thread, and a source that leaves it at 1
    /// silently removes the badge from every row.
    pub rows: Vec<Row>,
}

/// Which rows are wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    /// The mailbox being listed.
    pub mailbox: MailboxId,
    /// The page index, for the reply to be matched against.
    pub page: u32,
    /// The first row wanted, counted from the newest.
    pub offset: u32,
    /// How many rows to read.
    pub limit: u32,
}

/// The answer to a [`PageRequest`], awaited on the main thread.
///
/// Not `Send`: it is awaited by `glib::spawn_future_local`, and whatever
/// crosses to the runtime does so inside the implementation.
pub type PageFuture = Pin<Box<dyn Future<Output = Result<Page, String>>>>;

/// Where the message list's rows come from.
///
/// One method, because one is all the list needs. An implementation reads
/// `postio-storage`'s message repository off the UI thread and answers; a
/// test answers from a table.
pub trait MessageSource {
    /// Start reading `request`, and answer when the rows are in hand.
    fn fetch(&self, request: PageRequest) -> PageFuture;
}

/// What to call when a page cannot be read.
type ErrorHandler = Box<dyn Fn(String)>;

struct Inner {
    /// Weak, because the list owns the [`PageSource`] that owns this.
    list: glib::WeakRef<MessageList>,
    source: Rc<dyn MessageSource>,
    mailbox: Cell<Option<MailboxId>>,
    /// Bumped by every [`Feed::open`]. A reply from an older generation is
    /// answering a question nobody is asking any more.
    generation: Cell<u64>,
    total: Cell<u32>,
    errors: RefCell<Vec<ErrorHandler>>,
}

/// The `PageSource` the model holds.
///
/// A thin newtype rather than `Inner` itself, so `request` can take an owned
/// `Rc` into the future it spawns without `Inner` having to hold a weak
/// reference to itself.
struct Source(Rc<Inner>);

impl PageSource for Source {
    fn total(&self) -> u32 {
        self.0.total.get()
    }

    fn request(&self, page: u32) {
        self.0.clone().request(page);
    }
}

impl Inner {
    fn request(self: Rc<Self>, page: u32) {
        let Some(mailbox) = self.mailbox.get() else {
            return;
        };
        let generation = self.generation.get();
        let future = self.source.fetch(PageRequest {
            mailbox,
            page,
            offset: page * PAGE_SIZE,
            limit: PAGE_SIZE,
        });
        glib::spawn_future_local(async move {
            match future.await {
                Ok(answer) => self.deliver(generation, page, answer),
                Err(message) => self.fail(generation, message),
            }
        });
    }

    fn deliver(&self, generation: u64, page: u32, answer: Page) {
        if generation != self.generation.get() {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        // The count first: `deliver` clamps the page against it, so rows
        // delivered before the list knows how long it is would be dropped.
        self.total.set(answer.total);
        list.set_total(answer.total);
        list.deliver(page, answer.rows);
    }

    fn fail(&self, generation: u64, message: String) {
        if generation != self.generation.get() {
            return;
        }
        for handler in self.errors.borrow().iter() {
            handler(message.clone());
        }
    }

    fn showing(&self, mailbox: MailboxId) -> bool {
        self.mailbox.get() == Some(mailbox)
    }
}

/// The message list, fed.
///
/// Holds the source, owns the mailbox currently in view, and turns runtime
/// events into the smallest model update that is correct.
#[derive(Clone)]
pub struct Feed(Rc<Inner>);

impl Feed {
    /// Feed `list` from `source`. Shows nothing until [`open`](Self::open).
    pub fn new(list: &MessageList, source: Rc<dyn MessageSource>) -> Self {
        Feed(Rc::new(Inner {
            list: list.downgrade(),
            source,
            mailbox: Cell::new(None),
            generation: Cell::new(0),
            total: Cell::new(0),
            errors: RefCell::new(Vec::new()),
        }))
    }

    /// Show `mailbox`, discarding whatever the list was showing.
    ///
    /// Returns immediately: the first page is on its way, and until it lands
    /// the list is empty rather than wrong. There is no spinner, because a
    /// local read is not something to wait for — if this ever feels like a
    /// wait, the query is the bug.
    pub fn open(&self, mailbox: MailboxId) {
        let inner = &self.0;
        inner.generation.set(inner.generation.get() + 1);
        inner.mailbox.set(Some(mailbox));
        inner.total.set(0);
        if let Some(list) = inner.list.upgrade() {
            list.set_source(Rc::new(Source(inner.clone())));
        }
        // Asked for here rather than left to the view: the list is empty
        // until something says how long it is, and an empty list never asks
        // for a page. The reply caches page 0, so the view's own first
        // request finds it already there.
        inner.clone().request(0);
    }

    /// The mailbox in view, if any.
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.0.mailbox.get()
    }

    /// Called when a page cannot be read. The reason is the user's, not a log line.
    pub fn connect_error(&self, handler: impl Fn(String) + 'static) {
        self.0.errors.borrow_mut().push(Box::new(handler));
    }

    /// Apply one runtime event to the list.
    ///
    /// Everything it does not recognise it ignores, deliberately: the event
    /// stream carries the whole application, and a list that reacted to all
    /// of it would repaint on every keystroke in the composer.
    pub fn apply(&self, event: &Event) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        match event {
            // New mail lands at the top and every row shifts down. This is
            // an insertion, not a reload, which is what keeps the selection
            // on the message it was on and the scroll where it was.
            Event::NewMail { mailbox, messages } if inner.showing(*mailbox) => {
                list.inserted_at_top(messages.len() as u32);
            }
            // Flags, read state, labels: the rows are still the same rows in
            // the same order, so only the pages holding them are refetched.
            // `MessageList::deliver` replaces the data inside the existing
            // `GObject`, so nothing above rediscovers anything.
            Event::MessagesChanged { messages } => {
                let pages: BTreeSet<u32> = messages
                    .iter()
                    .filter_map(|message| list.page_of(*message))
                    .collect();
                for page in pages {
                    inner.clone().request(page);
                }
            }
            // The count moved, and so did every position after the gap.
            Event::MessagesRemoved { mailbox, .. } if inner.showing(*mailbox) => {
                self.reload();
            }
            // The order itself moved: a resync, a re-sort, a filter change.
            Event::MessageListChanged { mailbox } if inner.showing(*mailbox) => {
                self.reload();
            }
            _ => {}
        }
    }

    /// Drop everything cached and ask again, keeping the scroll position.
    ///
    /// The count corrects itself: every page carries the total, so the first
    /// reply back tells the list how long it now is.
    pub fn reload(&self) {
        let Some(list) = self.0.list.upgrade() else {
            return;
        };
        list.invalidate();
        // A list that shrank to nothing stops asking for pages, so the
        // reload has to ask once itself or an emptied mailbox would keep
        // showing the rows it used to have.
        self.0.clone().request(0);
    }
}
