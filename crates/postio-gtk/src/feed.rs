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
//! [`ListScope::reaction`] answers this, per scope, and both sides agree on
//! one rule: a list reacts to an event only when the event can change its
//! own membership or order, and it inserts at the top only when its own
//! order guarantees the new rows belong there. Everything else reloads
//! (`Feed::reload`, dropping everything cached and asking again). #773 is
//! the investigation this table closes.
//!
//! | Scope | [`Event::NewMail`] | [`Event::MessagesRemoved`] | [`Event::MessageListChanged`] | [`Event::MessagesChanged`] |
//! |---|---|---|---|---|
//! | [`ListScope::Mailbox`] | insert at top when the mailbox matches | reload when the mailbox matches | reload when the mailbox matches | refetch resident pages holding them |
//! | [`ListScope::Account`] | insert at top when the account matches | reload when the account matches | reload when the account matches | refetch resident pages holding them |
//! | [`ListScope::Flagged`] / [`ListScope::Snoozed`] | ignore — a delivery is neither flagged nor snoozed | reload when the account matches | reload when the account matches | **reload** when the account matches |
//! | a result set | ignore | ignore | ignore | refetch resident pages holding them |
//!
//! `Flagged`/`Snoozed` reloading on `MessagesChanged` rather than
//! refetching is the one cell that looks like the others and is not: for
//! them the flag or the snooze *is* the membership predicate, so a change
//! can remove a row the same event would repaint for a mailbox. A page
//! refetch cannot express a row leaving; only a reload can, because the
//! total moved. They are gated on the *account*, not the mailbox, because
//! they span every folder in one — the mailbox an event names carries no
//! information for them.
//!
//! [`ListScope::Thread`] never reaches a `Feed` at all: a drill-in issues
//! one direct [`MessageSource::fetch`] instead
//! (`postio_gtk::window::Window::open_thread`), so it has no event routing
//! to get wrong.
//!
//! An event naming a different mailbox, or a different account, is ignored
//! outright — a folder or a scope nobody is looking at costs nothing to be
//! wrong about until it is opened.

use std::cell::{Cell, RefCell};
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
use postio_model::{Arrival, Reaction};

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

/// Which messages a list is showing.
///
/// # Why the list is not keyed by a mailbox
///
/// "Flagged" is a folder in the sidebar and not a folder on the server: it is
/// a query over a role, and it has no [`MailboxId`] because there is no row
/// anywhere that it names. So what travels with a selection has to be the
/// *scope*, not an id — otherwise a smart folder can be drawn and cannot be
/// opened, which is a dead end wearing a folder's clothes.
///
/// `postio_model::ListScope`, not a spelling of its own: the view layer
/// already depends on `postio-model`, so there is no seam left to translate
/// across (#670). [`ListScope::mailbox`] is what `commands::mirror` reads to
/// tell app state which mailbox is open, and a smart folder must not claim
/// to be one.
pub use postio_model::ListScope;

/// Which rows are wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    /// The messages being listed.
    pub scope: ListScope,
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
    /// What the list is showing: one folder, or a role-scoped query.
    scope: Cell<Option<ListScope>>,
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
    /// Told whenever the list is pointed at a different scope.
    ///
    /// What the pane says about the rows depends on *which* scope they came
    /// from — an aggregate view answers ADR 0005 Q10's rule and a folder does
    /// not — so a scope change has to re-derive it. Nothing else did: the
    /// pane was refreshed on a status change and on rows arriving, and
    /// switching from a folder to the unified list is neither.
    opened: RefCell<Vec<Box<dyn Fn()>>>,
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
        let Some(scope) = self.scope.get() else {
            return;
        };
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let generation = list.generation();
        let future = self.source.fetch(PageRequest {
            scope,
            page,
            offset: page * PAGE_SIZE,
            limit: PAGE_SIZE,
        });
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: `MessageSource::fetch` is a trait method, and the trait's
            // contract is that what it returns is pollable on the main
            // context -- `postio-app` implements it by spawning the runtime
            // work and handing back a channel receive. A `MailBackend` future
            // must never be returned from it directly.
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
        let Some(list) = self.list.upgrade() else {
            return;
        };
        let generation = list.generation();
        let future = source.rows(ids[start..end].to_vec());
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: `MessageSource::rows` is a trait method, and the trait's
            // contract is that what it returns is pollable on the main
            // context -- `postio-app` implements it by spawning the runtime
            // work and handing back a channel receive. A `MailBackend` future
            // must never be returned from it directly.
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
        let Some(list) = self.list.upgrade() else {
            return;
        };
        list.deliver_for(generation, page, rows);
    }

    fn deliver(&self, generation: u64, page: u32, answer: Page) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        // Feed's own bookkeeping is generation-checked here, same as before:
        // `list.deliver_page` below checks again for the list's own state, but
        // a stale reply must not overwrite what `total`/`mailbox_total` will
        // hand back the next time this scope is reopened.
        if generation == list.generation() {
            self.total.set(answer.total);
            self.mailbox_total.set(answer.total);
        }
        list.deliver_page(generation, answer.total, page, answer.rows);
    }

    fn fail(&self, generation: u64, message: String) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        if generation != list.generation() {
            return;
        }
        for handler in self.errors.borrow().iter() {
            handler(message.clone());
        }
    }

    /// What the list showing `self.scope` does with one mailbox-shaped
    /// `arrival`.
    ///
    /// A result set has taken the list — however much it remembers which
    /// folder to go back to — behaves like [`ListScope::Mailbox`] for
    /// [`Arrival::MessagesChanged`] (the same rows in the same order;
    /// refetch the pages holding them) and ignores the other three, the
    /// scope underneath set aside until the result set is left: its
    /// membership is decided by the query, not by delivery order or a
    /// resync. Nothing has been opened yet answers [`Reaction::Ignore`] too.
    fn reaction(
        &self,
        arrival: Arrival,
        account: AccountId,
        mailbox: Option<MailboxId>,
    ) -> Reaction {
        if self.results.borrow().is_some() {
            return match arrival {
                Arrival::MessagesChanged => Reaction::Refetch,
                Arrival::NewMail | Arrival::MessagesRemoved | Arrival::MessageListChanged => {
                    Reaction::Ignore
                }
            };
        }
        self.scope
            .get()
            .map(|scope| scope.reaction(arrival, account, mailbox))
            .unwrap_or(Reaction::Ignore)
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
            scope: Cell::new(None),
            total: Cell::new(0),
            mailbox_total: Cell::new(0),
            results: RefCell::new(None),
            hits: RefCell::new(None),
            errors: RefCell::new(Vec::new()),
            opened: RefCell::new(Vec::new()),
            on_results: RefCell::new(Vec::new()),
        }))
    }

    /// Show `scope`, discarding whatever the list was showing.
    ///
    /// Returns immediately: the first page is on its way, and until it lands
    /// the list is empty rather than wrong. There is no spinner, because a
    /// local read is not something to wait for — if this ever feels like a
    /// wait, the query is the bug.
    pub fn open(&self, scope: ListScope) {
        let inner = &self.0;
        inner.scope.set(Some(scope));
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
        for handler in inner.opened.borrow().iter() {
            handler();
        }
    }

    /// Called whenever the list is pointed at a different scope.
    ///
    /// Separate from [`connect_results`](Self::connect_results): that one is
    /// about a result set arriving, this one about the list being aimed
    /// somewhere else, and a pane that reads the scope has to hear about the
    /// second even when the first never happens.
    pub fn connect_opened(&self, handler: impl Fn() + 'static) {
        self.0.opened.borrow_mut().push(Box::new(handler));
    }

    /// The mailbox in view, if the list is showing one.
    ///
    /// `None` in a smart folder as well as before anything is open. That is
    /// deliberate: `commands::mirror` feeds this to `AppState::open_mailbox`,
    /// and a role-scoped query is not a mailbox an action can be aimed at.
    /// See [`ListScope::mailbox`].
    pub fn mailbox(&self) -> Option<MailboxId> {
        self.0.scope.get().and_then(ListScope::mailbox)
    }

    /// What the list is showing, folder or query.
    pub fn scope(&self) -> Option<ListScope> {
        self.0.scope.get()
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
    /// of it would repaint on every keystroke in the composer. What each
    /// scope does with the four that remain is [`ListScope::reaction`]'s
    /// table, in the module docs above.
    pub fn apply(&self, event: &Event) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        match event {
            Event::NewMail {
                account,
                mailbox,
                messages,
                ..
            } => {
                if inner.reaction(Arrival::NewMail, *account, Some(*mailbox))
                    == Reaction::InsertAtTop
                {
                    list.inserted_at_top(messages.len() as u32);
                }
            }
            // Flags, read state, labels, a snooze: the same rows in the same
            // order for a mailbox, so only the pages holding them are
            // refetched -- `MessageList::deliver` replaces the data inside
            // the existing `GObject`, so nothing above rediscovers anything.
            // For `Flagged`/`Snoozed` the flag *is* the membership predicate,
            // so `ListScope::reaction` answers `Reload` there instead: a page
            // refetch cannot express a row leaving.
            Event::MessagesChanged { account, messages } => {
                match inner.reaction(Arrival::MessagesChanged, *account, None) {
                    Reaction::Refetch => {
                        for page in list.pages_holding(messages) {
                            inner.clone().request(page);
                        }
                    }
                    Reaction::Reload => self.reload(),
                    Reaction::Ignore | Reaction::InsertAtTop => {}
                }
            }
            // The count moved, and so did every position after the gap.
            Event::MessagesRemoved {
                account, mailbox, ..
            } => {
                if inner.reaction(Arrival::MessagesRemoved, *account, Some(*mailbox))
                    == Reaction::Reload
                {
                    self.reload();
                }
            }
            // The order itself moved: a resync, a re-sort, a filter change.
            Event::MessageListChanged { account, mailbox } => {
                if inner.reaction(Arrival::MessageListChanged, *account, Some(*mailbox))
                    == Reaction::Reload
                {
                    self.reload();
                }
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
/// [`ConnectionState::Failing`] carries a typed category — what *kind* of
/// help the account needs (ADR 0005 Q10) — but not prose. The prose travels
/// beside it as [`Event::Error`], so the tracker keeps the last one it saw
/// and promotes it the moment the connection starts failing. Leaving that
/// state clears it: a reason that outlived the failure it explained would be
/// worse than none.
#[derive(Clone, Debug, Default)]
pub struct SyncTracker {
    status: SyncStatus,
    /// The last error seen, waiting to explain a failure that may not come.
    reason: Option<String>,
}

/// One [`SyncTracker`] per account, so no account's server speaks for another.
///
/// Every status-bearing event names the account it is about, and a single
/// tracker threw that away: with two accounts configured, the status line
/// showed whichever server reported most recently. That is invisible with one
/// account, which is why it survived — and it is load-bearing for ADR 0005
/// Q10, whose whole subject is *which* account is not answering.
///
/// [`Event::Error`] is the exception, because it carries no account. It goes
/// to the account whose line is on screen, which is exactly what the single
/// tracker did with it; writing it down here makes it a decision rather than
/// an accident of which arm ran first.
#[derive(Clone, Debug, Default)]
pub struct Trackers {
    per_account: std::collections::BTreeMap<AccountId, SyncTracker>,
}

impl Trackers {
    /// Fold `event` in, routed to the account it names.
    ///
    /// `current` is the account whose status line is on screen, used only
    /// for the events that name none. Returns whether anything changed.
    pub fn apply(&mut self, event: &Event, current: Option<AccountId>) -> bool {
        let account = match event {
            Event::ConnectionChanged { account, .. }
            | Event::SyncProgress { account, .. }
            | Event::BackfillProgress { account, .. } => Some(*account),
            _ => current,
        };
        let Some(account) = account else {
            return false;
        };
        self.per_account.entry(account).or_default().apply(event)
    }

    /// What `account`'s line should say.
    ///
    /// An account nothing has been heard about is offline — the same default
    /// [`postio_core::AppState::connection`] gives, and for the same reason:
    /// silence is not a claim that the server is reachable.
    pub fn status(&self, account: AccountId) -> SyncStatus {
        self.per_account
            .get(&account)
            .map(|tracker| tracker.status().clone())
            .unwrap_or_default()
    }

    /// Fold `account`'s own folders' last-sync time into its tracker.
    ///
    /// Per account and not over the whole flat list: in section mode the
    /// sidebar reads every account's tree into one vector, and the newest
    /// `last_synced_at` in it belongs to whichever account synced most
    /// recently — which is exactly the cross-account confusion this type
    /// exists to end.
    pub fn note_last_sync(&mut self, account: AccountId, mailboxes: &[Mailbox]) -> bool {
        let theirs: Vec<Mailbox> = mailboxes
            .iter()
            .filter(|mailbox| mailbox.account_id == account)
            .cloned()
            .collect();
        self.per_account
            .entry(account)
            .or_default()
            .note_last_sync(&theirs)
    }

    /// The statuses of `accounts`, in the order given.
    ///
    /// The caller's order, because it is the sidebar's, which is the order
    /// the per-account hues are keyed to. An account nothing has been heard
    /// about still gets an entry: dropping it would be its own omission, in
    /// the one place whose subject is not omitting things.
    pub fn statuses(&self, accounts: &[AccountId]) -> Vec<(AccountId, SyncStatus)> {
        accounts
            .iter()
            .map(|account| (*account, self.status(*account)))
            .collect()
    }
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
                if matches!(state, ConnectionState::Failing { .. }) {
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
                // The body queue's number is deliberately *not* cleared here
                // (issue #316). The reasoning above is true for a list pass,
                // which really does end at a connection boundary — but a
                // backfill does not: it spans many IDLE cycles and
                // reconnects while it keeps running, so `ConnectionChanged`
                // fires constantly in the middle of one. Dropping the count
                // on every one of those left the line reading `idle` for as
                // long as it took the *next* body to settle and the 250 ms
                // floor on top of that, while a body was genuinely still on
                // the wire. `BackfillProgress` clears the count itself once
                // the queue actually drains — that is the boundary that
                // matters for this number, not a connection event.
            }
            Event::BackfillProgress {
                done,
                total,
                footprint,
                ..
            } => {
                self.status.backfill = Some((*done, *total));
                // Kept even when the queue drains below: the size of an
                // account's mail is true whether or not a backfill is
                // running, and the settings panel asks for it at a moment
                // that has nothing to do with one.
                if footprint.is_some() {
                    self.status.footprint = *footprint;
                }
                // Drained. Clear it rather than leaving `2000 of 2000` on
                // screen -- the same trap `SyncProgress` documents above,
                // and the same answer. `last_sync` is deliberately not
                // touched: it means a *list* pass completed, and a body
                // queue draining is not that.
                if done >= total {
                    self.status.backfill = None;
                }
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
                if matches!(self.status.state, ConnectionState::Failing { .. }) {
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
    /// Every account whose tree the sidebar is drawing, in the order the
    /// strip lists them (#185).
    ///
    /// Empty means the single-account shape: `account` alone, and the sidebar
    /// draws one flat folder list exactly as it always has. That is the
    /// common case and it costs one query, which is why this is a separate
    /// field rather than `account` becoming a `Vec` — a store with one
    /// account must not start paying for a loop it has no use for.
    sections: RefCell<Vec<AccountId>>,
    /// The folders as last read — including the synthetic ones — so picking
    /// one can name it without another round trip.
    mailboxes: RefCell<Vec<Mailbox>>,
    trackers: RefCell<Trackers>,
    generation: Cell<u64>,
    /// Whether a reload is already queued for this turn of the main loop.
    queued: Cell<bool>,
    statuses: RefCell<Vec<StatusHandler>>,
    loaded: RefCell<Vec<LoadedHandler>>,
    errors: RefCell<Vec<ErrorHandler>>,
}

impl FolderInner {
    fn reload_now(self: Rc<Self>) {
        let accounts: Vec<AccountId> = match self.sections.borrow().as_slice() {
            [] => self.account.get().into_iter().collect(),
            many => many.to_vec(),
        };
        if accounts.is_empty() {
            return;
        }
        let generation = self.generation.get();
        // One request per account, awaited in order and concatenated.
        // `Mailbox` carries its own `account_id`, so the sidebar can group a
        // flat list back into sections without a second shape to keep in
        // step — see `sidebar::folder_rows`.
        //
        // In order rather than joined: a folder list is a handful of rows
        // from a local table, and two accounts is two of those. Racing them
        // would buy nothing and would make the sidebar's order depend on
        // which query answered first, which is the one thing it must not do
        // (the hue is the position).
        let futures: Vec<_> = accounts
            .iter()
            .map(|account| self.source.mailboxes(*account))
            .collect();
        glib::spawn_future_local(async move {
            let mut all = Vec::new();
            for future in futures {
                // POSTIO-GLIB-SAFE: see the note on the single-account path
                // below -- `MailboxSource::mailboxes` returns something
                // pollable on the main context by contract.
                match future.await {
                    Ok(mailboxes) => all.extend(mailboxes),
                    Err(message) => {
                        if generation == self.generation.get() {
                            for handler in self.errors.borrow().iter() {
                                handler(message.clone());
                            }
                        }
                        return;
                    }
                }
            }
            self.arrived(generation, all);
        });
    }

    fn arrived(&self, generation: u64, mut mailboxes: Vec<Mailbox>) {
        if generation != self.generation.get() {
            return;
        }
        // Read the real folders before the synthetic one joins them. The
        // status line is about servers and syncs, and a query has neither —
        // a `last_synced_at` on it would be a claim about a sync that never
        // happened to anything.
        let moved = {
            let mut trackers = self.trackers.borrow_mut();
            let accounts: Vec<AccountId> = match self.sections.borrow().as_slice() {
                [] => self.account.get().into_iter().collect(),
                many => many.to_vec(),
            };
            // Every account, and deliberately not `any`: that short-circuits
            // on the first one that moved, and the rest would never be told
            // their own last-sync time at all.
            let mut moved = false;
            for account in accounts {
                moved |= trackers.note_last_sync(account, &mailboxes);
            }
            moved
        };
        // The smart folder goes in here, before anything else sees the list,
        // so the sidebar keeps drawing exactly what it is handed and learns
        // nothing about folders the server does not have. The next one — a
        // saved search — joins the same way.
        if let Some(account) = self.account.get() {
            let flagged = mailboxes.iter().map(|folder| folder.counts.flagged).sum();
            let snoozed = mailboxes.iter().map(|folder| folder.counts.snoozed).sum();
            mailboxes.push(flagged_folder(account, flagged));
            mailboxes.push(snoozed_folder(account, snoozed));
        }
        self.sidebar.set_mailboxes(&mailboxes);
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
        let status = match self.account.get() {
            Some(account) => self.trackers.borrow().status(account),
            None => SyncStatus::default(),
        };
        self.sidebar.set_status(status.clone());
        for handler in self.statuses.borrow().iter() {
            handler(&status);
        }
    }
}

/// The id the synthetic "Flagged" row is keyed by in the sidebar.
///
/// # Why a sentinel is safe here, and where it must not go
///
/// The sidebar keys its rows by [`MailboxId`], so a folder it draws needs one
/// even when nothing in the database corresponds to it. Negative is
/// unambiguous: SQLite rowids start at 1 and `MailboxId::UNASSIGNED` is 0, so
/// this can never collide with a real folder.
///
/// It is contained to exactly one hop. The sidebar hands it back on a click,
/// [`Folders::scope_of`] turns it into a [`ListScope`], and from there the
/// list, the store and app state all deal in scopes. It must never reach a
/// query — `MessageSet::InMailbox { mailbox: -1 }` matches nothing, silently
/// — nor `Command::Move`, whose destination is a foreign key.
const FLAGGED_ROW: MailboxId = MailboxId::new(-1);

/// The sidebar's "Flagged" row: a query wearing a folder's clothes.
///
/// Not a mailbox the server has, and deliberately not one the store has
/// either. `path` is empty because there is nothing to `SELECT`, and
/// `last_synced_at` stays `None` because a query is never out of date.
fn flagged_folder(account: AccountId, flagged: u32) -> Mailbox {
    let mut folder = Mailbox::new(account, "", None);
    folder.id = FLAGGED_ROW;
    folder.role = postio_model::mailbox::MailboxRole::Flagged;
    folder.counts = postio_model::mailbox::MailboxCounts {
        total: flagged,
        unread: 0,
        flagged,
        snoozed: 0,
    };
    folder
}

/// The id the synthetic "Snoozed" row is keyed by — see [`FLAGGED_ROW`] for
/// why a sentinel is safe and where it must not go. A second, distinct
/// negative value: the two synthetic rows must never collide with each
/// other any more than with a real folder.
const SNOOZED_ROW: MailboxId = MailboxId::new(-2);

/// The sidebar's "Snoozed" row — [`flagged_folder`]'s own shape, for the
/// other view every ordinary scope hides its rows from.
fn snoozed_folder(account: AccountId, snoozed: u32) -> Mailbox {
    let mut folder = Mailbox::new(account, "", None);
    folder.id = SNOOZED_ROW;
    folder.role = postio_model::mailbox::MailboxRole::Snoozed;
    folder.counts = postio_model::mailbox::MailboxCounts {
        total: snoozed,
        unread: 0,
        flagged: 0,
        snoozed,
    };
    folder
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
            sections: RefCell::new(Vec::new()),
            mailboxes: RefCell::new(Vec::new()),
            trackers: RefCell::new(Trackers::default()),
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
        inner.sections.borrow_mut().clear();
        inner.sidebar.set_account(address);
        inner.clone().reload_now();
    }

    /// Draw every account's tree at once, as sections (#185).
    ///
    /// `selected` stays the account a verb without a folder is aimed at —
    /// the sections are what is *visible*, not what is *current*, and
    /// conflating them would make opening a section move the scope under
    /// somebody who only wanted to look.
    pub fn open_sections(&self, accounts: &[AccountId], selected: AccountId, address: &str) {
        let inner = &self.0;
        inner.generation.set(inner.generation.get() + 1);
        inner.account.set(Some(selected));
        *inner.sections.borrow_mut() = accounts.to_vec();
        inner.sidebar.set_account(address);
        inner.clone().reload_now();
    }

    /// Which reading of the folder tree this is.
    ///
    /// Bumped by [`open`](Self::open) and
    /// [`open_sections`](Self::open_sections) -- the two things that change
    /// *which* account's folders are on screen -- and by nothing else. A
    /// reload for a `MailboxesChanged` keeps the generation it had.
    ///
    /// That is the distinction #813 needed and could not get from the feed:
    /// a smart folder on screen looks the same whether the user chose it or
    /// `GtkListBox` auto-selected its sentinel before the real folders
    /// arrived, but "have I already picked a folder for *this* tree" tells
    /// the two apart.
    pub fn generation(&self) -> u64 {
        self.0.generation.get()
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

    /// What picking the sidebar row `id` should show.
    ///
    /// The one place a sidebar row becomes a query. Everything downstream —
    /// the list, the store, app state — deals in [`ListScope`], so this is
    /// where [`FLAGGED_ROW`] stops being an id and starts being what it
    /// actually meant.
    pub fn scope_of(&self, id: MailboxId) -> ListScope {
        match self.0.account.get() {
            Some(account) if id == FLAGGED_ROW => ListScope::Flagged(account),
            Some(account) if id == SNOOZED_ROW => ListScope::Snoozed(account),
            _ => ListScope::Mailbox(id),
        }
    }

    /// Where the account stands with its server, right now.
    pub fn status(&self) -> SyncStatus {
        match self.0.account.get() {
            Some(account) => self.0.trackers.borrow().status(account),
            None => SyncStatus::default(),
        }
    }

    /// Every account the sidebar is drawing, and where each stands with its
    /// server — what an aggregate view needs to say which one is away.
    ///
    /// In single-account mode this is the one account, so a caller does not
    /// have to know which shape the sidebar is in.
    pub fn statuses(&self) -> Vec<(AccountId, SyncStatus)> {
        let inner = &self.0;
        let accounts: Vec<AccountId> = match inner.sections.borrow().as_slice() {
            [] => inner.account.get().into_iter().collect(),
            many => many.to_vec(),
        };
        inner.trackers.borrow().statuses(&accounts)
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
        // Real folders only. The synthetic rows are in this list too --
        // `arrived` appends Flagged and Snoozed before anything else sees it,
        // and it does so even when the account has no folders yet -- so
        // `first()` on an unsynced account was the Flagged sentinel, and this
        // answered "open Flagged" to a question that means "which folder".
        //
        // That is #813's second half. The caller opened the sentinel, counted
        // its turn as spent, and never opened the inbox when the first sync
        // finally delivered the tree; `postio-app`'s `e2e` sees it as a
        // window that syncs three messages and lists none.
        let real = || mailboxes.iter().filter(|mailbox| mailbox.id.get() > 0);
        real()
            .find(|mailbox| mailbox.role == postio_model::mailbox::MailboxRole::Inbox)
            .or_else(|| real().next())
            .map(|mailbox| mailbox.id)
    }

    /// Called when the folders cannot be read.
    pub fn connect_error(&self, handler: impl Fn(String) + 'static) {
        self.0.errors.borrow_mut().push(Box::new(handler));
    }

    /// Apply one runtime event to the sidebar.
    pub fn apply(&self, event: &Event) {
        let inner = &self.0;
        if inner
            .trackers
            .borrow_mut()
            .apply(event, inner.account.get())
        {
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
    /// Whatever else the composition root put on screen — see
    /// [`Feeds::connect_event`]. Shared, so a clone of these feeds delivers
    /// to the same consumers rather than to a copy that nobody registered
    /// with.
    others: Rc<RefCell<Vec<EventHandler>>>,
}

/// What [`Feeds::connect_event`] holds.
type EventHandler = Box<dyn Fn(&Event)>;

impl Feeds {
    /// The two panes this crate builds, plus room for the ones it does not.
    pub fn new(messages: Feed, folders: Folders) -> Self {
        Feeds {
            messages,
            folders,
            others: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Take another consumer of the event stream.
    ///
    /// The sidebar and the list are fed from inside this crate because their
    /// contents *are* this crate's. The reading pane is not: what a body is
    /// and how one is read from the store live in `postio-app`, which is why
    /// [`Event::BodyLoaded`] had no consumer for as long as it did (#396) —
    /// it is addressed to a surface this file cannot name.
    ///
    /// So the seam is left open rather than the dependency inverted: whoever
    /// assembles the application still hands [`apply`](Self::apply) every
    /// event, and everything on screen is still fed by that one call.
    pub fn connect_event(&self, handler: impl Fn(&Event) + 'static) {
        self.others.borrow_mut().push(Box::new(handler));
    }

    /// Apply one event to everything that cares about it.
    pub fn apply(&self, event: &Event) {
        self.messages.apply(event);
        self.folders.apply(event);
        for handler in self.others.borrow().iter() {
            handler(event);
        }
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

    /// Issue #74: the backfill's progress reached nobody, so the longest
    /// phase of a first sync drew `idle`.
    #[test]
    fn a_backfill_moves_the_status_line_and_then_gets_out_of_the_way() {
        let mut tracker = SyncTracker::new();
        assert!(tracker.apply(&Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Online,
        }));
        assert_eq!(tracker.status().backfill, None);

        assert!(
            tracker.apply(&Event::BackfillProgress {
                account: AccountId::new(1),
                done: 412,
                total: 2000,
                // Nothing measured yet: these predate the field, and they are
                // about the counter, not the size.
                footprint: None,
            }),
            "the status changed and the tracker said it had not"
        );
        assert_eq!(tracker.status().backfill, Some((412, 2000)));

        // Drained. It must clear itself the way `SyncProgress` does, or the
        // line reads `downloading` for as long as the account stays up.
        tracker.apply(&Event::BackfillProgress {
            account: AccountId::new(1),
            done: 2000,
            total: 2000,
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
            footprint: None,
        });
        assert_eq!(
            tracker.status().backfill,
            None,
            "a queue that has drained is not a backfill in progress"
        );
        assert_eq!(tracker.status().lines(Instant::now()).0, "idle · imap");
    }

    /// Issue #316: `ConnectionChanged` cleared `backfill` unconditionally, on
    /// the reasoning that "the engine announces a connection state at the
    /// boundaries" — true for a list pass, which really does end there, but
    /// not for a backfill, which spans many IDLE cycles and reconnects while
    /// it keeps running. A connection blip mid-backfill erased the count the
    /// line was showing and the sidebar read `idle` while a body was still
    /// on the wire — seen live at the same moment the reading pane showed
    /// "Downloading this message" for the selected message.
    #[test]
    fn a_connection_announcement_mid_backfill_does_not_erase_its_count() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::BackfillProgress {
            account: account(),
            done: 412,
            total: 2000,
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
            footprint: None,
        });
        assert_eq!(
            tracker.status().lines(Instant::now()).0,
            "downloading · imap"
        );

        // The engine announces an IDLE cycle or a reconnect mid-backfill the
        // same way it announces anything else on the link: a connection
        // state, here `Online` again rather than a drain.
        tracker.apply(&connection(ConnectionState::Online));

        assert_eq!(
            tracker.status().backfill,
            Some((412, 2000)),
            "a connection announcement mid-backfill must not erase its count"
        );
        assert_eq!(
            tracker.status().lines(Instant::now()).0,
            "downloading · imap",
            "the status line must not claim idle while a body is still in flight"
        );
    }

    #[test]
    fn a_backfill_does_not_pretend_to_be_a_sync() {
        // `last_sync` is what "last sync 4h" reads, and it means a *list*
        // pass completed. A backfill finishing is not that, and moving it
        // would date the mailbox from the wrong event.
        let mut tracker = SyncTracker::new();
        tracker.apply(&Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Online,
        });
        let before = tracker.status().last_sync;
        tracker.apply(&Event::BackfillProgress {
            account: AccountId::new(1),
            done: 2000,
            total: 2000,
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
            footprint: None,
        });
        assert_eq!(
            tracker.status().last_sync,
            before,
            "a drained body queue is not a completed sync"
        );
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
        tracker.apply(&connection(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        }));
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
        tracker.apply(&connection(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        }));
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
            account: account(),
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

#[cfg(test)]
mod trackers_tests {
    use super::*;

    const WORK: AccountId = AccountId::new(1);
    const HOME: AccountId = AccountId::new(2);

    #[test]
    fn each_account_keeps_its_own_connection_state() {
        // The bug this type exists to fix: one tracker folded every
        // account's `ConnectionChanged` into one status, last writer wins,
        // so with two accounts the sidebar's line showed whichever server
        // happened to report most recently.
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::ConnectionChanged {
                account: WORK,
                state: ConnectionState::Online,
            },
            Some(WORK),
        );
        trackers.apply(
            &Event::ConnectionChanged {
                account: HOME,
                state: ConnectionState::Offline,
            },
            Some(WORK),
        );

        assert_eq!(
            trackers.status(WORK).state,
            ConnectionState::Online,
            "Home going offline said nothing about Work"
        );
        assert_eq!(trackers.status(HOME).state, ConnectionState::Offline);
    }

    #[test]
    fn an_account_nothing_has_been_heard_about_is_working_locally() {
        // The same default `AppState::connection` gives, and for the same
        // reason: silence is not a claim that the server is reachable.
        let trackers = Trackers::default();
        assert_eq!(trackers.status(WORK).state, ConnectionState::Offline);
    }

    #[test]
    fn progress_lands_on_the_account_it_names_and_no_other() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::SyncProgress {
                account: HOME,
                done: 3,
                total: 10,
            },
            Some(WORK),
        );
        assert_eq!(trackers.status(HOME).progress, Some((3, 10)));
        assert_eq!(
            trackers.status(WORK).progress,
            None,
            "Work is not syncing and its line must not say it is"
        );
    }

    #[test]
    fn an_error_carries_no_account_so_it_lands_on_the_one_in_view() {
        // `Event::Error` has no account field. Routing it to the account
        // whose line is on screen is exactly what the single tracker did,
        // so this is no worse -- and it is written down here rather than
        // left as an accident of which arm ran.
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::Error {
                message: "the server refused the password".to_owned(),
            },
            Some(WORK),
        );
        trackers.apply(
            &Event::ConnectionChanged {
                account: WORK,
                state: ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
            },
            Some(WORK),
        );
        assert_eq!(
            trackers.status(WORK).detail.as_deref(),
            Some("the server refused the password")
        );
        assert_eq!(
            trackers.status(HOME).detail,
            None,
            "an error with no account named must not be attributed to one"
        );
    }

    #[test]
    fn statuses_are_reported_for_the_accounts_asked_for_in_that_order() {
        // The order is the caller's -- the sidebar's -- because that is the
        // order the hues are keyed to, and an account absent from the map
        // still has to appear rather than silently drop out of the banner.
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::ConnectionChanged {
                account: HOME,
                state: ConnectionState::Offline,
            },
            Some(WORK),
        );
        let named = trackers.statuses(&[WORK, HOME]);
        assert_eq!(named.len(), 2);
        assert_eq!(named[0].0, WORK);
        assert_eq!(named[0].1.state, ConnectionState::Offline, "never heard of");
        assert_eq!(named[1].0, HOME);
        assert_eq!(named[1].1.state, ConnectionState::Offline);
    }
}
