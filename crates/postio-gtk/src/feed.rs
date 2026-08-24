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
use std::time::Instant;

use chrono::{DateTime, Utc};
use gtk::glib;
use gtk::prelude::*;
use postio_core::{ConnectionState, Event};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::Mailbox;

use crate::list::{MessageList, PAGE_SIZE, PageSource, Row};
use crate::sidebar::SyncStatus;

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

/// The answer to a request for a result set's rows.
///
/// No total: unlike a mailbox, a result set already knows how long it is —
/// the ids *are* the answer, and they all arrived at once.
pub type RowsFuture = Pin<Box<dyn Future<Output = Result<Vec<Row>, String>>>>;

/// Where the rows for a set of search hits come from.
///
/// Separate from [`MessageSource`] because a result set is not a window over
/// a mailbox. The hits are an explicit ranked list of ids that may span
/// folders, so there is no offset to read from, no mailbox to read it in, and
/// no count to carry back. What the two do share is the crossing: this is
/// read off the UI thread and awaited with `glib::spawn_future_local`, the
/// same as every other page.
pub trait ResultSource {
    /// Read the rows for `ids`, in the order given.
    ///
    /// The order is the ranking, and it is the caller's, not the store's: a
    /// repository asked for a set of ids will happily answer in whatever
    /// order its index found them, which would silently re-sort the results
    /// by date and lose the thing the search was for.
    fn rows(&self, ids: Vec<MessageId>) -> RowsFuture;
}

/// What to call when a page cannot be read.
type ErrorHandler = Box<dyn Fn(String)>;

/// What to call when a result set takes the list, with how many hits it holds.
type ResultHandler = Box<dyn Fn(u32)>;

struct Inner {
    /// Weak, because the list owns the [`PageSource`] that owns this.
    list: glib::WeakRef<MessageList>,
    source: Rc<dyn MessageSource>,
    mailbox: Cell<Option<MailboxId>>,
    /// Bumped by every [`Feed::open`]. A reply from an older generation is
    /// answering a question nobody is asking any more.
    generation: Cell<u64>,
    total: Cell<u32>,
    /// How long the *mailbox* was, last time one was read.
    ///
    /// Kept apart from `total` so that leaving a result set can put the
    /// list back at its real length in the same turn it stops showing hits.
    /// Without it the list would go to the hit count, then to zero, then
    /// back — and the scroll offset the window restores would be measured
    /// against a scroller that had collapsed in between.
    mailbox_total: Cell<u32>,
    /// The hits in view, ranked, or `None` when a mailbox is in view.
    results: RefCell<Option<Rc<Vec<MessageId>>>>,
    /// Where a result set's rows come from. `None` in a window that has no
    /// search wired to it, which is the only reason this is an `Option`.
    hits: RefCell<Option<Rc<dyn ResultSource>>>,
    errors: RefCell<Vec<ErrorHandler>>,
    /// Told when a result set takes the list, with how many hits it holds.
    ///
    /// The `Feed` is what changes mode, so it is what says so. Anything that
    /// has to follow the list into a search — the column header counting
    /// results rather than naming a folder, whatever remembers the mailbox to
    /// come back to — hangs off this rather than off the event, so it cannot
    /// be told about a result set the list did not actually take.
    on_results: RefCell<Vec<ResultHandler>>,
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
        if self.results.borrow().is_some() {
            self.request_hits(page);
            return;
        }
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

    /// The same request, for a page of a result set rather than of a mailbox.
    fn request_hits(self: Rc<Self>, page: u32) {
        let ids = self.results.borrow().clone();
        let source = self.hits.borrow().clone();
        let (Some(ids), Some(source)) = (ids, source) else {
            return;
        };
        let start = (page * PAGE_SIZE) as usize;
        if start >= ids.len() {
            return;
        }
        // The last page of a result set is short, and asking for the ids it
        // does not have would make the source answer for messages nobody
        // matched.
        let end = ids.len().min(start + PAGE_SIZE as usize);
        let generation = self.generation.get();
        let future = source.rows(ids[start..end].to_vec());
        glib::spawn_future_local(async move {
            match future.await {
                Ok(rows) => self.deliver_hits(generation, page, rows),
                Err(message) => self.fail(generation, message),
            }
        });
    }

    /// Hand over a page of hits.
    ///
    /// No count to set, unlike [`deliver`](Self::deliver): the result set's
    /// length was known the moment the ids arrived.
    fn deliver_hits(&self, generation: u64, page: u32, rows: Vec<Row>) {
        if generation != self.generation.get() {
            return;
        }
        let Some(list) = self.list.upgrade() else {
            return;
        };
        list.deliver(page, rows);
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
        self.mailbox_total.set(answer.total);
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

    /// Whether the list is showing `mailbox` — which a list showing search
    /// results is not, however much it remembers which folder to go back to.
    ///
    /// This is the guard on every mailbox-shaped event at once. `NewMail`
    /// is the one that matters: the mailbox got longer, the result set did
    /// not, and inserting at the top of a result set would put a message
    /// that does not match the query above one that does.
    fn showing(&self, mailbox: MailboxId) -> bool {
        self.results.borrow().is_none() && self.mailbox.get() == Some(mailbox)
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
            mailbox_total: Cell::new(0),
            results: RefCell::new(None),
            hits: RefCell::new(None),
            errors: RefCell::new(Vec::new()),
            on_results: RefCell::new(Vec::new()),
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
        inner.mailbox_total.set(0);
        // Opening a folder is leaving the results, if there were any: the
        // sidebar is a way out of a search as much as `Esc` is.
        *inner.results.borrow_mut() = None;
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

    /// Where a result set's rows come from.
    ///
    /// Set once, when the application is assembled. A `Feed` without one
    /// shows mailboxes and ignores [`Event::SearchResults`] — which is
    /// correct for a window that has no search wired to it, and is why this
    /// is a setter rather than an argument to [`new`](Self::new).
    pub fn set_result_source(&self, source: Rc<dyn ResultSource>) {
        *self.0.hits.borrow_mut() = Some(source);
    }

    /// Whether the list is showing search hits rather than a mailbox.
    pub fn showing_results(&self) -> bool {
        self.0.results.borrow().is_some()
    }

    /// Called when a result set takes the list, with how many hits it holds.
    ///
    /// Not called by [`close_results`](Self::close_results): leaving is a
    /// gesture whoever wired it made on purpose, and it already knows.
    pub fn connect_results(&self, handler: impl Fn(u32) + 'static) {
        self.0.on_results.borrow_mut().push(Box::new(handler));
    }

    /// Show `messages` — the hits, most relevant first — instead of the
    /// mailbox.
    ///
    /// The mailbox is remembered, not left: `Esc` goes back to it, and until
    /// then it is still the folder the user is in. Only the ids are held; the
    /// rows are read a page at a time like any other, so a query matching
    /// forty thousand messages costs forty thousand ids and one page of mail.
    pub fn show_results(&self, messages: Vec<MessageId>) {
        let inner = &self.0;
        if inner.hits.borrow().is_none() {
            return;
        }
        inner.generation.set(inner.generation.get() + 1);
        let total = messages.len() as u32;
        *inner.results.borrow_mut() = Some(Rc::new(messages));
        inner.total.set(total);
        if let Some(list) = inner.list.upgrade() {
            list.set_source(Rc::new(Source(inner.clone())));
        }
        inner.clone().request(0);
        // After the list is the result set, not before: a handler that reads
        // the list back — to remember what it was showing, or to size
        // something against it — must not see the mailbox it just replaced.
        for handler in inner.on_results.borrow().iter() {
            handler(total);
        }
    }

    /// Put the mailbox back. Returns whether there were results to leave.
    ///
    /// The count does not pass through zero on the way: see
    /// `Inner::mailbox_total`. The rows themselves are re-read, because they
    /// were dropped when the result set took the list — but the list is the
    /// right length from this call, which is what lets the window restore a
    /// scroll offset without waiting for a read.
    pub fn close_results(&self) -> bool {
        let inner = &self.0;
        if inner.results.borrow().is_none() {
            return false;
        }
        inner.generation.set(inner.generation.get() + 1);
        *inner.results.borrow_mut() = None;
        inner.total.set(inner.mailbox_total.get());
        if let Some(list) = inner.list.upgrade() {
            list.set_source(Rc::new(Source(inner.clone())));
        }
        inner.clone().request(0);
        true
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
            // The hits are the list now. Handled here rather than by whoever
            // ran the search because this is where the list's source lives,
            // and because it makes every route to a search -- the box, a
            // saved query, a command -- land in one place.
            Event::SearchResults { messages, .. } => {
                self.show_results(messages.clone());
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

// ── The sidebar ──────────────────────────────────────────────────────────

/// The answer to a request for an account's folders.
pub type MailboxFuture = Pin<Box<dyn Future<Output = Result<Vec<Mailbox>, String>>>>;

/// Where the sidebar's folders come from.
///
/// The same crossing as [`MessageSource`], for the same reason: the mailbox
/// repository is on the other side of a crate boundary this crate may not
/// cross. `Mailbox` itself is a `postio-model` type, so unlike the message
/// list there is nothing to map — the sidebar shows the domain's own record.
pub trait MailboxSource {
    /// Read `account`'s folders, with their counts as of now.
    fn mailboxes(&self, account: AccountId) -> MailboxFuture;
}

/// The status line, folded out of the runtime's events.
///
/// Pure, and separate from the widget, because "what does the status line
/// say when the connection drops mid-resync" is a question worth answering
/// without a display in the loop.
///
/// # Where the failure reason comes from
///
/// [`ConnectionState::Failing`] carries no reason of its own — deliberately,
/// so `postio-core` need not change when the sync engine's state machine
/// does. The reason travels beside it as [`Event::Error`], so the tracker
/// keeps the last one it saw and promotes it the moment the connection
/// starts failing. Leaving that state clears it: a reason that outlived the
/// failure it explained would be worse than none.
#[derive(Clone, Debug, Default)]
pub struct SyncTracker {
    status: SyncStatus,
    /// The last error seen, waiting to explain a failure that may not come.
    reason: Option<String>,
}

impl SyncTracker {
    /// A tracker that has heard nothing yet: offline, never synced.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the status line should say.
    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    /// Fold `event` in. Returns whether the status line changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        let before = self.status.clone();
        match event {
            Event::ConnectionChanged { state, .. } => {
                self.status.state = *state;
                if *state == ConnectionState::Failing {
                    self.status.detail = self.reason.clone();
                } else {
                    // Connected, connecting or deliberately offline: whatever
                    // went wrong before is no longer what is happening.
                    self.status.detail = None;
                    self.reason = None;
                }
                // A pass's progress belongs to that pass. The engine announces
                // a connection state at the *boundaries* — a pass starting or
                // finishing, or the link itself moving — and never between two
                // batches, so any of them means the number on screen is no
                // longer being made.
                //
                // Including `Online`, which is the case that matters: a pass
                // ends by moving the tracker to idle, and idle is announced as
                // `Online`. `SyncProgress` only clears itself when `done`
                // reaches `total`, and `total` is `UIDNEXT - 1` — an upper
                // bound that expunged messages leave gaps in, so a pass can
                // finish having never reached it. Leaving `Online` alone left
                // the line reading `syncing 89%` on a folder that had finished,
                // for as long as the account stayed connected.
                self.status.progress = None;
            }
            Event::SyncProgress { done, total, .. } => {
                self.status.progress = Some((*done, *total));
                // A resync that reached its own total is a sync that
                // finished, and that is when "last sync" moved.
                if done >= total {
                    self.status.last_sync = Some(Instant::now());
                    self.status.progress = None;
                }
            }
            Event::Error { message } => {
                self.reason = Some(message.clone());
                if self.status.state == ConnectionState::Failing {
                    self.status.detail = Some(message.clone());
                }
            }
            _ => return false,
        }
        self.status != before
    }

    /// Record when this account last completed a sync, from its folders.
    ///
    /// [`SyncStatus::last_sync`] is an [`Instant`] on purpose: the line shows
    /// an *age*, and an age that jumped when the system clock was corrected
    /// would be worse than no age at all. The stored time is wall-clock, so
    /// the conversion happens here, once, at the boundary.
    pub fn note_last_sync(&mut self, mailboxes: &[Mailbox]) -> bool {
        let Some(latest) = mailboxes.iter().filter_map(|m| m.last_synced_at).max() else {
            return false;
        };
        let converted = to_instant(latest, Utc::now(), Instant::now());
        if converted.is_some() && self.status.last_sync.is_none() {
            self.status.last_sync = converted;
            return true;
        }
        false
    }
}

/// A wall-clock time as a point on the monotonic clock, relative to `now`.
///
/// `None` for a time in the future or further back than the process has been
/// running: neither can be expressed as an `Instant`, and inventing one would
/// put a fabricated age on the status line.
pub fn to_instant(at: DateTime<Utc>, now: DateTime<Utc>, monotonic: Instant) -> Option<Instant> {
    let age = now.signed_duration_since(at).to_std().ok()?;
    monotonic.checked_sub(age)
}

/// What to call when the status line moves.
type StatusHandler = Box<dyn Fn(&SyncStatus)>;

/// What to call when the folders have been read.
type LoadedHandler = Box<dyn Fn(&[Mailbox])>;

struct FolderInner {
    sidebar: crate::sidebar::Sidebar,
    source: Rc<dyn MailboxSource>,
    account: Cell<Option<AccountId>>,
    /// The folders as last read, so picking one can name it without
    /// another round trip.
    mailboxes: RefCell<Vec<Mailbox>>,
    tracker: RefCell<SyncTracker>,
    generation: Cell<u64>,
    /// Whether a reload is already queued for this turn of the main loop.
    queued: Cell<bool>,
    statuses: RefCell<Vec<StatusHandler>>,
    loaded: RefCell<Vec<LoadedHandler>>,
    errors: RefCell<Vec<ErrorHandler>>,
}

impl FolderInner {
    fn reload_now(self: Rc<Self>) {
        let Some(account) = self.account.get() else {
            return;
        };
        let generation = self.generation.get();
        let future = self.source.mailboxes(account);
        glib::spawn_future_local(async move {
            match future.await {
                Ok(mailboxes) => self.arrived(generation, mailboxes),
                Err(message) => {
                    if generation == self.generation.get() {
                        for handler in self.errors.borrow().iter() {
                            handler(message.clone());
                        }
                    }
                }
            }
        });
    }

    fn arrived(&self, generation: u64, mailboxes: Vec<Mailbox>) {
        if generation != self.generation.get() {
            return;
        }
        self.sidebar.set_mailboxes(&mailboxes);
        let moved = self.tracker.borrow_mut().note_last_sync(&mailboxes);
        *self.mailboxes.borrow_mut() = mailboxes;
        if moved {
            self.publish();
        }
        let mailboxes = self.mailboxes.borrow().clone();
        for handler in self.loaded.borrow().iter() {
            handler(&mailboxes);
        }
    }

    fn publish(&self) {
        let status = self.tracker.borrow().status().clone();
        self.sidebar.set_status(status.clone());
        for handler in self.statuses.borrow().iter() {
            handler(&status);
        }
    }
}

/// The sidebar, fed.
///
/// Owns the account's folders and the status line, and keeps both in step
/// with the runtime.
#[derive(Clone)]
pub struct Folders(Rc<FolderInner>);

impl Folders {
    /// Feed `sidebar` from `source`. Shows nothing until [`open`](Self::open).
    pub fn new(sidebar: &crate::sidebar::Sidebar, source: Rc<dyn MailboxSource>) -> Self {
        let folders = Folders(Rc::new(FolderInner {
            sidebar: sidebar.clone(),
            source,
            account: Cell::new(None),
            mailboxes: RefCell::new(Vec::new()),
            tracker: RefCell::new(SyncTracker::new()),
            generation: Cell::new(0),
            queued: Cell::new(false),
            statuses: RefCell::new(Vec::new()),
            loaded: RefCell::new(Vec::new()),
            errors: RefCell::new(Vec::new()),
        }));
        // Offline, never synced — which is the truth until something says
        // otherwise, and is what the sidebar should say meanwhile.
        folders.0.publish();
        folders
    }

    /// Show `account`'s folders, with `address` as the kicker.
    pub fn open(&self, account: AccountId, address: &str) {
        let inner = &self.0;
        inner.generation.set(inner.generation.get() + 1);
        inner.account.set(Some(account));
        inner.sidebar.set_account(address);
        inner.clone().reload_now();
    }

    /// The folders as last read.
    pub fn mailboxes(&self) -> Vec<Mailbox> {
        self.0.mailboxes.borrow().clone()
    }

    /// One folder by id, if it has been read.
    pub fn mailbox(&self, id: MailboxId) -> Option<Mailbox> {
        self.0
            .mailboxes
            .borrow()
            .iter()
            .find(|mailbox| mailbox.id == id)
            .cloned()
    }

    /// Where the account stands with its server, right now.
    pub fn status(&self) -> SyncStatus {
        self.0.tracker.borrow().status().clone()
    }

    /// Called whenever the status line moves — for the list pane's own
    /// named states, which read the same status the sidebar does.
    pub fn connect_status(&self, handler: impl Fn(&SyncStatus) + 'static) {
        self.0.statuses.borrow_mut().push(Box::new(handler));
    }

    /// Called every time the folders have been read.
    ///
    /// How the window knows which mailbox to open on startup: the folders
    /// are not there yet when [`open`](Self::open) returns.
    pub fn connect_loaded(&self, handler: impl Fn(&[Mailbox]) + 'static) {
        self.0.loaded.borrow_mut().push(Box::new(handler));
    }

    /// The folder to show when nothing has been picked yet.
    ///
    /// The inbox, or the first folder there is. A mail client that opens
    /// into no folder has asked the user a question before saying hello.
    pub fn default_mailbox(&self) -> Option<MailboxId> {
        let mailboxes = self.0.mailboxes.borrow();
        mailboxes
            .iter()
            .find(|mailbox| mailbox.role == postio_model::mailbox::MailboxRole::Inbox)
            .or_else(|| mailboxes.first())
            .map(|mailbox| mailbox.id)
    }

    /// Called when the folders cannot be read.
    pub fn connect_error(&self, handler: impl Fn(String) + 'static) {
        self.0.errors.borrow_mut().push(Box::new(handler));
    }

    /// Apply one runtime event to the sidebar.
    pub fn apply(&self, event: &Event) {
        let inner = &self.0;
        if inner.tracker.borrow_mut().apply(event) {
            inner.publish();
        }
        let ours = |account: &AccountId| inner.account.get() == Some(*account);
        match event {
            // The tree itself moved: renamed, created, unsubscribed.
            Event::MailboxesChanged { account } if ours(account) => self.reload(),
            // Counts move with read state and with mail arriving or leaving.
            // Which mailbox is irrelevant — the sidebar shows all of them.
            Event::MessagesChanged { .. }
            | Event::MessagesRemoved { .. }
            | Event::NewMail { .. }
            | Event::MessageListChanged { .. } => self.reload(),
            _ => {}
        }
    }

    /// Read the folders again, at most once per turn of the main loop.
    ///
    /// Coalesced on purpose: a resync emits `MessagesChanged` in bursts, and
    /// a sidebar that re-read every folder's counts per event would spend a
    /// sync hammering the database to draw the same numbers.
    pub fn reload(&self) {
        let inner = &self.0;
        if inner.account.get().is_none() || inner.queued.replace(true) {
            return;
        }
        let inner = inner.clone();
        glib::idle_add_local_once(move || {
            inner.queued.set(false);
            inner.reload_now();
        });
    }
}

/// Both panes, fed from the same runtime.
///
/// The one thing whoever assembles the application has to hold: hand it
/// every [`Event`] and the sidebar, the status line and the message list all
/// stay in step.
#[derive(Clone)]
pub struct Feeds {
    /// The message list.
    pub messages: Feed,
    /// The folders and the status line.
    pub folders: Folders,
}

impl Feeds {
    /// Apply one event to everything that cares about it.
    pub fn apply(&self, event: &Event) {
        self.messages.apply(event);
        self.folders.apply(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::mailbox::MailboxRole;

    fn account() -> AccountId {
        AccountId::new(1)
    }

    fn connection(state: ConnectionState) -> Event {
        Event::ConnectionChanged {
            account: account(),
            state,
        }
    }

    #[test]
    fn the_status_line_follows_a_connection_all_the_way_round() {
        let mut tracker = SyncTracker::new();
        assert_eq!(tracker.status().state, ConnectionState::Offline);
        assert_eq!(tracker.status().last_sync, None);

        assert!(tracker.apply(&connection(ConnectionState::Connecting)));
        assert_eq!(tracker.status().state, ConnectionState::Connecting);

        assert!(tracker.apply(&connection(ConnectionState::Online)));
        assert!(tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 40,
            total: 100,
        }));
        assert_eq!(tracker.status().progress, Some((40, 100)));

        // A resync that reaches its own total is a sync that finished.
        assert!(tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 100,
            total: 100,
        }));
        assert_eq!(tracker.status().progress, None);
        assert!(tracker.status().last_sync.is_some());
    }

    #[test]
    fn a_failing_connection_carries_the_reason_it_was_given() {
        let mut tracker = SyncTracker::new();
        // The reason arrives beside the state change, not inside it.
        tracker.apply(&Event::Error {
            message: "the server rejected the password".to_string(),
        });
        tracker.apply(&connection(ConnectionState::Failing));
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the server rejected the password")
        );

        // And an error that arrives while already failing replaces it.
        tracker.apply(&Event::Error {
            message: "the certificate expired".to_string(),
        });
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the certificate expired")
        );

        // Recovering clears it: a reason that outlived its failure is worse
        // than no reason.
        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(tracker.status().detail, None);

        // And it does not come back on the next unrelated failure.
        tracker.apply(&connection(ConnectionState::Failing));
        assert_eq!(tracker.status().detail, None);
    }

    #[test]
    fn a_dropped_connection_stops_reporting_progress_it_is_not_making() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 3,
            total: 90,
        });
        tracker.apply(&connection(ConnectionState::Offline));
        assert_eq!(tracker.status().progress, None, "syncing 3% while offline");
    }

    #[test]
    fn a_pass_that_ends_short_of_its_own_total_stops_reporting_a_percentage() {
        // `total` is `UIDNEXT - 1`: an upper bound, not a promise, because
        // expunged messages leave gaps in the UID space. So a pass can finish
        // having fetched everything there is and still never reach it, and the
        // last report before it ended is a percentage below 100.
        //
        // The pass ending is announced as idle, which reaches the tracker as
        // `Online`. If that did not clear the number, the line would read
        // `syncing 89% · imap` for as long as the account stayed connected —
        // on a folder with nothing left to sync.
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 89,
            total: 100,
        });
        assert_eq!(tracker.status().progress, Some((89, 100)), "mid-pass");

        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(
            tracker.status().progress,
            None,
            "the pass finished; there is no percentage to be a percentage of"
        );
    }

    #[test]
    fn events_the_status_line_is_not_about_change_nothing() {
        let mut tracker = SyncTracker::new();
        assert!(!tracker.apply(&Event::MailboxesChanged { account: account() }));
        assert!(!tracker.apply(&Event::BodyLoaded {
            message: postio_model::ids::MessageId::new(1),
        }));
    }

    #[test]
    fn the_last_sync_age_comes_off_the_monotonic_clock() {
        let now = Utc::now();
        let monotonic = Instant::now();

        // An hour ago is an hour ago, whatever the wall clock does next.
        let hour = to_instant(now - chrono::Duration::hours(1), now, monotonic)
            .expect("an hour is expressible");
        assert!((monotonic.duration_since(hour).as_secs() as i64 - 3600).abs() <= 1);

        // A time in the future is not an age, and is refused rather than
        // turned into one.
        assert_eq!(
            to_instant(now + chrono::Duration::hours(1), now, monotonic),
            None
        );
    }

    #[test]
    fn folders_report_when_the_account_last_synced() {
        let synced = |id: i64, at: Option<DateTime<Utc>>| {
            let mut mailbox = Mailbox::new(account(), "INBOX", Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = MailboxRole::Inbox;
            mailbox.last_synced_at = at;
            mailbox
        };
        let now = Utc::now();

        let mut tracker = SyncTracker::new();
        assert!(!tracker.note_last_sync(&[synced(1, None)]), "never synced");
        assert_eq!(tracker.status().last_sync, None);

        // The newest of them wins: one stale folder does not make the
        // account look stale.
        assert!(tracker.note_last_sync(&[
            synced(1, Some(now - chrono::Duration::days(2))),
            synced(2, Some(now - chrono::Duration::seconds(12))),
        ]));
        let age = Instant::now().saturating_duration_since(tracker.status().last_sync.unwrap());
        assert!(age.as_secs() <= 13, "the age came out as {age:?}");
    }
}
