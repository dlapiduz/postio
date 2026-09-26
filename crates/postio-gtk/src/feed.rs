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
//! What the list does with a runtime event — insert at the top, refetch
//! the pages holding the rows, reload, or nothing — is
//! [`postio_ui::paging::Paging`]'s table, per scope, shared with the macOS
//! frontend; this module only carries the answer out to the widget.
//! [`ListScope::Thread`] never reaches a `Feed` at all: a drill-in issues
//! one direct [`MessageSource::fetch`] instead
//! (`postio_gtk::window::Window::open_thread`), so it has no event routing
//! to get wrong.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_ui::paging::{Fetch, Paging, Plan};

use crate::list::{MessageList, PageSource, Row};
use crate::sidebar::{SidebarChoice, SyncStatus};
use postio_ui::sidebar::ViewCounts;

/// One page of a mailbox, as the runtime answered it.
///
/// [`Row::thread_count`] is expected to be real here — the badge in the
/// canvas is a count of the thread, and a source that leaves it at 1
/// silently removes the badge from every row.
pub type Page = postio_ui::paging::Page<Row>;

/// Which rows are wanted. The shared spelling, so `postio-app`'s adapter
/// and the macOS boundary ask the store for the same thing.
pub use postio_ui::paging::PageRequest;

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

    /// The rows this scope's list shows for these messages, in the shape
    /// its pages use, so a change that names ids can be patched into the
    /// rows on screen rather than answered by re-reading their page
    /// (#1607). A source that reads pages only answers with an error and
    /// the feed re-reads the page, which is what this default says.
    fn rows_in(&self, scope: postio_model::ListScope, ids: Vec<MessageId>) -> RowsFuture {
        let _ = (scope, ids);
        Box::pin(async { Err("this source reads pages, not rows".to_owned()) })
    }

    /// These messages left `mailbox`, and the list is about to react. Said
    /// before the reaction, so a source that keeps a count for the folder
    /// can adjust it by what left rather than pay it again in front of the
    /// first row (#1607). A source with nothing to keep does nothing, which
    /// is this default.
    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let _ = (mailbox, messages);
    }
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
    /// What the list is showing, what a page of it means and what an event
    /// does to it — the policy, shared with the macOS frontend.
    paging: RefCell<Paging>,
    total: Cell<u32>,
    /// How long the *mailbox* was, last time one was read.
    ///
    /// Kept apart from `total` so that leaving a result set can put the
    /// list back at its real length in the same turn it stops showing hits.
    /// Without it the list would go to the hit count, then to zero, then
    /// back — and the scroll offset the window restores would be measured
    /// against a scroller that had collapsed in between.
    mailbox_total: Cell<u32>,
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

/// How many pages of a mailbox the feed has asked the store for.
///
/// "Never load a whole mailbox into memory" is a product promise, and this is
/// the number that keeps it honest: a folder switch should cost a screenful of
/// rows and their neighbours, not a mailbox. Counted rather than timed, for the
/// reason [`crate::list::emissions`] gives.
pub fn fetches() -> u64 {
    FETCHES.load(std::sync::atomic::Ordering::Relaxed)
}
static FETCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Inner {
    /// Ask for `page` of whatever is in view — a mailbox by offset, or a
    /// result set by its ids — and deliver the answer when it lands.
    fn request(self: Rc<Self>, page: u32) {
        FETCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some(fetch) = self.paging.borrow().fetch_for(page) else {
            // Nothing to read -- a result set with no hits has no page 0.
            // A list keeping the last rows on screen until this page lands
            // would otherwise wait for good; the answer is that there is none.
            if let Some(list) = self.list.upgrade() {
                list.give_up(list.generation(), page);
            }
            return;
        };
        let Some(list) = self.list.upgrade() else {
            return;
        };
        // Remembered whichever way it was asked for, so a second ask in the
        // same turn -- a change and a reload landing together -- finds it
        // pending and reads it once (#1607). A scroll's ask remembered it
        // already; this is idempotent for that one.
        list.note_pending(page);
        let generation = list.generation();
        match fetch {
            Fetch::Scope(request) => {
                let future = self.source.fetch(request);
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: `MessageSource::fetch` is a trait method, and the trait's
                    // contract is that what it returns is pollable on the main
                    // context -- `postio-app` implements it by spawning the runtime
                    // work and handing back a channel receive. A `MailBackend` future
                    // must never be returned from it directly.
                    match future.await {
                        Ok(answer) => self.deliver(generation, page, answer),
                        Err(message) => self.fail(generation, page, message),
                    }
                });
            }
            Fetch::Hits { ids, .. } => {
                let Some(source) = self.hits.borrow().clone() else {
                    return;
                };
                let future = source.rows(ids);
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: `ResultSource::rows` is a trait method, and the trait's
                    // contract is that what it returns is pollable on the main
                    // context -- `postio-app` implements it by spawning the runtime
                    // work and handing back a channel receive. A `MailBackend` future
                    // must never be returned from it directly.
                    match future.await {
                        Ok(rows) => self.deliver_hits(generation, page, rows),
                        Err(message) => self.fail(generation, page, message),
                    }
                });
            }
        }
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

    /// A page that came back an error rather than rows.
    ///
    /// Two things have to happen and only one of them used to. The handlers
    /// raise a banner, which tells a person; `abandon_page` lets the page be
    /// asked for again, which is the only thing that can actually put the
    /// rows back. `MessageList` asks once per page and never again until it
    /// is answered or abandoned -- the rule that keeps a 100,000-message
    /// folder cheap -- so a failure that answered neither left those fifty
    /// rows as placeholders for the rest of the session.
    ///
    /// It does not retry here. The page becomes askable and the next repaint
    /// that needs a row in it asks, which is the ordinary path and cannot
    /// become a spin against a store that is failing every read.
    fn fail(self: Rc<Self>, generation: u64, page: u32, message: String) {
        let Some(list) = self.list.upgrade() else {
            return;
        };
        if generation != list.generation() {
            return;
        }
        // Ask again, up to a bound. Abandoning alone is not enough: the row
        // objects already handed to the view for those positions are returned
        // from `MessageList::row_at` without consulting the window, so nothing
        // would ever ask a second time on its own. The page has to be pushed.
        if self.paging.borrow_mut().retry(page) {
            list.abandon_page(generation, page);
            Rc::clone(&self).request(page);
            // Quiet while it is still trying. A read that collided with a
            // write and succeeds on the next ask is not something to tell
            // somebody about -- the retry is the answer, and a banner that
            // appears and is immediately wrong is worse than none.
            return;
        }

        // Out of attempts: now it is worth saying, and it is said once --
        // and the list stops waiting for a page that is not coming, or a
        // refresh holding its other pages for this one would hold them for
        // good.
        for handler in self.errors.borrow().iter() {
            handler(message.clone());
        }
        list.give_up(generation, page);
    }
}

/// The message list, fed.
///
/// Holds the source, owns the mailbox currently in view, and turns runtime
/// events into the smallest model update that is correct.
#[derive(Clone)]
pub struct Feed(Rc<Inner>);

/// A callback's non-owning reference to a message feed.
#[derive(Clone)]
pub(crate) struct WeakFeed(std::rc::Weak<Inner>);

impl WeakFeed {
    pub(crate) fn upgrade(&self) -> Option<Feed> {
        self.0.upgrade().map(Feed)
    }
}

impl Feed {
    pub(crate) fn downgrade(&self) -> WeakFeed {
        WeakFeed(Rc::downgrade(&self.0))
    }

    /// Feed `list` from `source`. Shows nothing until [`open`](Self::open).
    pub fn new(list: &MessageList, source: Rc<dyn MessageSource>) -> Self {
        Feed(Rc::new(Inner {
            list: list.downgrade(),
            source,
            paging: RefCell::new(Paging::default()),
            total: Cell::new(0),
            mailbox_total: Cell::new(0),
            hits: RefCell::new(None),
            errors: RefCell::new(Vec::new()),
            opened: RefCell::new(Vec::new()),
            on_results: RefCell::new(Vec::new()),
        }))
    }

    /// Tell the list which folders are inboxes: the tree the sidebar just
    /// read, every account it draws.
    ///
    /// Unified is the inboxes (#1692), so mail moving in any other folder
    /// cannot move one of its rows, and knowing which is which is what lets
    /// a sync of the Archive leave the view alone rather than re-read it.
    /// The sidebar's synthetic rows are not folders and are left out.
    pub fn set_folders(&self, mailboxes: &[Mailbox]) {
        self.0.paging.borrow_mut().set_folders(
            mailboxes
                .iter()
                .filter(|mailbox| mailbox.id.get() > 0)
                .map(|mailbox| (mailbox.id, mailbox.role == MailboxRole::Inbox)),
        );
    }

    /// Show `scope`, discarding whatever the list was showing.
    ///
    /// Returns immediately: the first page is on its way, and until it lands
    /// the list goes on showing what it was showing
    /// ([`MessageList::replace_source`]) rather than a screenful of
    /// skeletons. There is no spinner, because a local read is not
    /// something to wait for — if this ever feels like a wait, the query is
    /// the bug.
    pub fn open(&self, scope: ListScope) {
        let inner = &self.0;
        // Opening a folder is leaving the results, if there were any: the
        // sidebar is a way out of a search as much as `Esc` is.
        inner.paging.borrow_mut().open(scope);
        inner.total.set(0);
        inner.mailbox_total.set(0);
        if let Some(list) = inner.list.upgrade() {
            list.replace_source(Rc::new(Source(inner.clone())), false);
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
        self.0.paging.borrow().mailbox()
    }

    /// What the list is showing, folder or query.
    pub fn scope(&self) -> Option<ListScope> {
        self.0.paging.borrow().scope()
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
        self.0.paging.borrow().showing_results()
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
        let total = inner.paging.borrow_mut().show_results(messages);
        inner.total.set(total);
        if let Some(list) = inner.list.upgrade() {
            // The mailbox is kept whole underneath, so `Esc` puts it back
            // without a read.
            list.replace_source(Rc::new(Source(inner.clone())), true);
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
        if !inner.paging.borrow_mut().close_results() {
            return false;
        }
        inner.total.set(inner.mailbox_total.get());
        let Some(list) = inner.list.upgrade() else {
            return true;
        };
        // The mailbox the results covered, rows and all, in one step: no
        // read before it is back, and the scroll offset the window restores
        // next has the mailbox's length to land in. Then a refresh, which
        // re-reads only what is on screen and moves only what changed while
        // the search was up.
        if list.restore(Rc::new(Source(inner.clone()))) {
            list.refresh();
        } else {
            list.replace_source(Rc::new(Source(inner.clone())), false);
            inner.clone().request(0);
        }
        true
    }

    /// Called when a page cannot be read. The reason is the user's, not a log line.
    pub fn connect_error(&self, handler: impl Fn(String) + 'static) {
        self.0.errors.borrow_mut().push(Box::new(handler));
    }

    /// Apply one runtime event to the list.
    ///
    /// What each scope does with an event is [`Paging::plan`]'s table; this
    /// carries the answer to the model. A refetch re-reads only the pages
    /// holding the rows — `MessageList::deliver` replaces the data inside
    /// the existing `GObject`, so nothing above rediscovers anything — and a
    /// reload drops everything cached and asks again.
    pub fn apply(&self, event: &Event) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        // The hits are the list now. Handled here rather than by whoever
        // ran the search because this is where the list's source lives,
        // and because it makes every route to a search -- the box, a
        // saved query, a command -- land in one place.
        if let Event::SearchResults { messages, .. } = event {
            self.show_results(messages.clone());
            return;
        }
        // A removal from the folder on screen is told to the source before the
        // list reacts, so the read that follows can keep the folder's count
        // by subtracting what left instead of paying it again in front of the
        // first row (#1607). Only the folder on screen: a removal elsewhere
        // is learnt by that folder's own witness when it is next opened, and
        // telling the source about it would queue a fact nothing reads.
        if let Event::MessagesRemoved {
            mailbox, messages, ..
        } = event
            && inner.paging.borrow().scope() == Some(postio_model::ListScope::Mailbox(*mailbox))
        {
            inner.source.note_removed(*mailbox, messages.clone());
        }
        let plan = inner.paging.borrow().plan(event);
        match plan {
            Plan::Ignore => {}
            // Not `inserted_at_top`, which dropped every page and drew the
            // new rows as skeletons: a refresh reads the new rows first and
            // then inserts them, with their contents, where they belong.
            Plan::InsertAtTop(_) => self.reload(),
            Plan::Refetch(messages) => self.refetch(messages),
            Plan::Reload => match event {
                Event::MessagesRemoved {
                    mailbox, messages, ..
                } if inner.paging.borrow().scope()
                    == Some(postio_model::ListScope::Mailbox(*mailbox))
                    && list.all_resident(messages) =>
                {
                    self.remove_or_reload(*mailbox, messages.clone());
                }
                _ => self.reload(),
            },
        }
    }

    /// Take rows that left the folder on screen out where they stand, or
    /// reload when that cannot be done exactly (#1607).
    ///
    /// A reload rebuilt every row's widget, dropped every seek mark and read
    /// page 0 again, to take out rows the list was holding. What has to be
    /// known first is whether each row really left: a conversation that
    /// still has a member here stays, drawn from another message, so the
    /// store is asked for the rows these ids have now. None at all is the
    /// case this is for -- every one of them gone -- and anything else, or a
    /// list that moved while the question was out, reloads as before.
    fn remove_or_reload(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        let future = inner
            .source
            .rows_in(postio_model::ListScope::Mailbox(mailbox), messages.clone());
        let generation = list.generation();
        let feed = self.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: `MessageSource::rows_in`, under the contract
            // `refetch` states above.
            let still_here = future.await;
            let Some(list) = feed.0.list.upgrade() else {
                return;
            };
            let removed = generation == list.generation()
                && matches!(still_here, Ok(ref rows) if rows.is_empty())
                && list.remove_in_place(&messages);
            if !removed {
                feed.reload();
            }
        });
    }

    /// A change named these messages. Their rows are patched in place from
    /// a by-id read, and only a row that read could not describe costs its
    /// page again (#1607). Ids no resident row carries cost nothing now: a
    /// change to a message off screen is learnt when its page is next read,
    /// which is what re-reading its page would have done anyway.
    fn refetch(&self, messages: &[MessageId]) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        let resident: Vec<MessageId> = messages
            .iter()
            .copied()
            .filter(|id| !list.pages_holding(&[*id]).is_empty())
            .collect();
        if resident.is_empty() {
            return;
        }
        let Some(scope) = inner.paging.borrow().scope() else {
            self.refetch_pages(&resident);
            return;
        };
        let future = inner.source.rows_in(scope, resident.clone());
        let generation = list.generation();
        let feed = self.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: `MessageSource::rows_in` is a trait method
            // under the same contract as `fetch`: what it returns is pollable
            // on the main context -- `postio-app` spawns the runtime work and
            // hands back a channel receive, and the default is a ready error.
            let outcome = future.await;
            let Some(list) = feed.0.list.upgrade() else {
                return;
            };
            if generation != list.generation() {
                // The list moved on to another scope or was reloaded whole;
                // whatever it shows now was read after the change.
                return;
            }
            let mut answered = std::collections::HashSet::new();
            match outcome {
                Ok(rows) => {
                    for row in rows {
                        let id = row.id;
                        if list.update_row(row) {
                            answered.insert(id);
                        }
                    }
                }
                Err(reason) => {
                    tracing::debug!(%reason, "no rows by id; re-reading their pages");
                }
            }
            let unanswered: Vec<MessageId> = resident
                .into_iter()
                .filter(|id| !answered.contains(id))
                .collect();
            feed.refetch_pages(&unanswered);
        });
    }

    /// Re-read the pages holding these messages, once each, skipping a page
    /// already on its way.
    fn refetch_pages(&self, messages: &[MessageId]) {
        let inner = &self.0;
        let Some(list) = inner.list.upgrade() else {
            return;
        };
        for page in list.pages_holding(messages) {
            if !list.is_pending(page) {
                inner.clone().request(page);
            }
        }
    }

    /// Re-read what is on screen and move only what changed, keeping the
    /// scroll position and every other row. See [`MessageList::refresh`].
    ///
    /// The count corrects itself: every page carries the total, so the
    /// refresh's reply tells the list how long it now is -- and a list with
    /// nothing on screen reads the top, so an emptied mailbox does not keep
    /// the rows it used to have.
    pub fn reload(&self) {
        let Some(list) = self.0.list.upgrade() else {
            return;
        };
        list.refresh();
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

    /// What the sidebar draws beside Drafts and the Outbox.
    ///
    /// Separate from [`mailboxes`](Self::mailboxes) because the Outbox is not
    /// one: it has no row to carry a count, and the Drafts badge needs a
    /// number the cached column deliberately does not hold (spec 003 T066).
    ///
    /// **Defaults to `None`, meaning "I do not know"** — not to zero.
    ///
    /// The difference matters. Zero is an answer: it says this account has no
    /// drafts, and the sidebar acts on it by replacing what the Drafts row
    /// shows. A fixture that has never heard of drafts is not saying that, and
    /// treating its silence as zero empties the badge of a folder with mail in
    /// it. `None` leaves the cached counts alone.
    fn draft_counts(&self, _account: AccountId) -> DraftCountsFuture {
        Box::pin(async { Ok(None) })
    }
}

/// The answer to a request for an account's draft counts.
pub type DraftCountsFuture = Pin<Box<dyn Future<Output = Result<Option<ViewCounts>, String>>>>;

pub use postio_ui::status::{SyncTracker, Trackers, to_instant};

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
    /// The folders as last read — including the view rows — so picking one
    /// can name it without another round trip.
    mailboxes: RefCell<Vec<Mailbox>>,
    /// What the sidebar draws beside Drafts and the Outbox, as last read.
    ///
    /// Held beside the folders rather than derived from them: the Outbox is
    /// not a mailbox, so no folder's cached count adds up to it, and what
    /// Drafts should show excludes what is on its way, which `total_count`
    /// deliberately does not. Read in the same pass as the folders so the
    /// sidebar redraws once, with both.
    ///
    /// `None` until a source has actually answered. Not `ViewCounts::default()`:
    /// the trait's default answers "nothing in flight", which is true for a
    /// source with no drafts and is *not* a true answer for how many Drafts
    /// holds -- and overwriting a real cached total with a default zero empties
    /// the badge of a folder that has mail in it.
    drafts: Cell<Option<ViewCounts>>,
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
        // Built here, beside the folder reads, and awaited below for the same
        // reason they are: the future has to be made before the main context
        // starts polling, or `check-runtime-crossings` is right that this is a
        // runtime-dependent await on the glib loop.
        let counting = self
            .account
            .get()
            .map(|account| self.source.draft_counts(account));
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
            // Alongside the folders, in the same pass: the sidebar redraws
            // once with both, rather than drawing an Outbox-less column and
            // then correcting itself. A failure here is not worth abandoning
            // the folder list over -- the sidebar draws, without an Outbox
            // row, which is what it did before this existed.
            if let Some(counting) = counting {
                // POSTIO-GLIB-SAFE: a channel receive, like the folder reads
                // above -- `MailboxSource::draft_counts` returns something
                // pollable on the main context by the same contract.
                if let Ok(Some(counts)) = counting.await {
                    self.drafts.set(Some(counts));
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
        // The view rows go in here, before anything else sees the list, so
        // the sidebar keeps drawing exactly what it is handed and learns
        // nothing about folders the server does not have. *Which* rows and
        // what they hold is `postio_ui::sidebar`'s answer, not this file's —
        // that is what makes the macOS sidebar draw the same ones.
        if let Some(account) = self.account.get() {
            let counts = postio_ui::sidebar::ViewCounts {
                flagged: mailboxes.iter().map(|folder| folder.counts.flagged).sum(),
                snoozed: mailboxes.iter().map(|folder| folder.counts.snoozed).sum(),
                // Summed over the account's folders like the two above, but
                // asked for rather than derived: the Outbox is not a mailbox
                // and has no cached column to sum.
                outbox: self.drafts.get().map_or(0, |counts| counts.outbox),
                drafts: 0,
                attention: 0,
            };
            // The Drafts row's two numbers, which no cached column holds:
            // what Drafts shows excludes what is on its way, and nothing on a
            // mailbox row counts what has stopped and is waiting for somebody.
            if let Some(drafts) = self.drafts.get() {
                for mailbox in mailboxes
                    .iter_mut()
                    .filter(|m| m.role == MailboxRole::Drafts)
                {
                    mailbox.counts.total = drafts.drafts;
                    mailbox.counts.attention = drafts.attention;
                }
            }
            mailboxes.extend(postio_ui::sidebar::view_rows(account, &mailboxes, counts));
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

/// The sidebar, fed.
///
/// Owns the account's folders and the status line, and keeps both in step
/// with the runtime.
#[derive(Clone)]
pub struct Folders(Rc<FolderInner>);

/// A callback's non-owning reference to the folder feed.
#[derive(Clone)]
pub(crate) struct WeakFolders(std::rc::Weak<FolderInner>);

impl WeakFolders {
    pub(crate) fn upgrade(&self) -> Option<Folders> {
        self.0.upgrade().map(Folders)
    }
}

impl Folders {
    pub(crate) fn downgrade(&self) -> WeakFolders {
        WeakFolders(Rc::downgrade(&self.0))
    }

    /// Feed `sidebar` from `source`. Shows nothing until [`open`](Self::open).
    pub fn new(sidebar: &crate::sidebar::Sidebar, source: Rc<dyn MailboxSource>) -> Self {
        let folders = Folders(Rc::new(FolderInner {
            sidebar: sidebar.clone(),
            source,
            account: Cell::new(None),
            sections: RefCell::new(Vec::new()),
            mailboxes: RefCell::new(Vec::new()),
            drafts: Cell::new(None),
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
    /// where a [`SidebarChoice::View`] stops being a row and starts being
    /// what it actually meant.
    pub fn scope_of(&self, choice: SidebarChoice) -> ListScope {
        match (choice, self.0.account.get()) {
            (SidebarChoice::Folder(id), _) => ListScope::Mailbox(id),
            (SidebarChoice::View(role), Some(account)) => match role {
                MailboxRole::Flagged => ListScope::Flagged(account),
                MailboxRole::Snoozed => ListScope::Snoozed(account),
                MailboxRole::Outbox => ListScope::Outbox(account),
                // No other role reaches here: `view_rows` builds exactly those
                // three, and a row with an id is a `Folder` above. The account
                // is the one safe answer -- a superset of what was asked for,
                // never a scope that silently matches nothing and reads as an
                // empty folder.
                _ => ListScope::Account(account),
            },
            // A view with no account in scope: nothing to narrow to.
            (SidebarChoice::View(_), None) => ListScope::Unified,
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
        // Every account the sidebar draws: the one account, or each section
        // of the unified tree.
        let drawn =
            |account: &AccountId| ours(account) || inner.sections.borrow().contains(account);
        match event {
            // The tree itself moved: renamed, created, unsubscribed.
            Event::MailboxesChanged { account } if ours(account) => self.reload(),
            // Counts move with read state and with mail arriving or leaving.
            // Which mailbox is irrelevant — the sidebar shows all of them —
            // but which account is not: another account's sync burst used to
            // re-read these folders for numbers it could not have moved
            // (#1607).
            Event::MessagesChanged { account, .. }
            | Event::MessagesRemoved { account, .. }
            | Event::NewMail { account, .. }
            | Event::MessageListChanged { account, .. }
                if drawn(account) =>
            {
                self.reload()
            }
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
