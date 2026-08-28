//! The message list's model: a `GListModel` windowed over paged storage.
//!
//! docs/PRODUCT.md §18 is a hard requirement — a mailbox is never loaded into memory.
//! A `GtkListView` over this model asks for the rows it is about to draw and
//! nothing else, so a 100,000-message folder costs the same as a 50-message
//! one: a few hundred rows resident, and the rest a page request away.
//!
//! # How the pieces fit
//!
//! The view layer speaks no SQL, so the model does not fetch anything itself.
//! It says *which page it needs* through a [`PageSource`] and waits; whoever
//! implements that source — a repository call marshalled onto the tokio
//! runtime and back through the `postio-core` bridge — calls
//! [`MessageList::deliver`] on the main thread when the rows arrive. Until
//! then the positions in that page answer with a placeholder [`MessageRow`],
//! and the row widget draws a skeleton.
//!
//! Inverting it this way is also what makes the model testable without a
//! database, a display or a runtime: a fake source records what was asked for
//! and the test decides when to answer.
//!
//! # The paging and generation bookkeeping lives in `postio-ui` (ADR 0019 Q5a)
//!
//! [`postio_ui::list::ListWindow`] owns the resident pages, the LRU eviction
//! and the generation counter — the half of this a second frontend must not
//! re-derive. What stays here is exactly what GTK's own contract demands:
//! `MessageRow`'s `GObject` identity, `items_changed` emission (including
//! what an insertion at the top means to a scroll anchor), and the
//! `reading`/[`hold`](MessageList::hold) re-entrancy guard below —
//! `GListModel::item()` must not be mutated mid-call, which is a GTK rule
//! with no `NSTableView` equivalent, so `ListWindow` must never be reached
//! while it is set.
//!
//! # What it costs
//!
//! * [`CACHE_PAGES`] pages resident, evicted least-recently-used. Scrolling
//!   the length of a huge folder does not grow that number.
//! * One request per page, ever, until it is evicted — [`PageSource::request`]
//!   is never called twice for a page that is already on its way.
//! * A page either side of the one being read, prefetched, so scrolling at
//!   speed does not stutter on a page boundary.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::list::{ListRow, ListWindow, Lookup};

/// Rows per page.
pub use postio_ui::list::PAGE_SIZE;

/// Pages kept in memory. Everything past this is evicted least-recently-used.
pub use postio_ui::list::CACHE_PAGES;

/// What the message list shows for one message.
///
/// The view layer's own row, deliberately: `postio-storage` has a struct with
/// these fields, and depending on it would drag `rusqlite` into the frontend,
/// which is the one thing CI forbids here. It carries no body and no headers —
/// the reading pane loads those when a row is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Local id. The row's identity, across reloads and updates.
    pub id: MessageId,
    /// The thread it belongs to, for drill-in and the count badge.
    pub thread: Option<ThreadId>,
    /// Who it is from.
    pub from: Option<EmailAddress>,
    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// The snippet under the subject.
    pub preview: Option<String>,
    /// When the server received it; the list's sort key.
    pub received_at: DateTime<Utc>,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it carries `\Flagged`.
    pub flagged: bool,
    /// Whether it has been replied to.
    pub answered: bool,
    /// Whether it is a draft.
    pub draft: bool,
    /// Whether it has an attachment, for the paperclip.
    pub has_attachments: bool,
    /// How many messages are in its thread; the badge appears above one.
    pub thread_count: u32,
    /// Everyone who has written in the conversation, in first-seen order.
    ///
    /// **Empty on a message row, and that is how the two are told apart.** A
    /// folder shows one row per conversation (ADR 0015) and a query view
    /// shows messages; a row with participants stands for a conversation, so
    /// its sender line names the people in it rather than one sender, and the
    /// verbs act on the whole thread.
    ///
    /// All of them, elided when drawn: which names survive the width is a
    /// drawing decision, and the row is the only thing that knows how much
    /// room there is.
    pub participants: Vec<EmailAddress>,
}

impl Row {
    /// Whether this row stands for a whole conversation rather than one
    /// message.
    ///
    /// The one test, so nothing can disagree about what a thread row is.
    pub fn is_thread(&self) -> bool {
        !self.participants.is_empty()
    }
}

/// Where the rows come from.
///
/// The model never blocks on this. [`request`](PageSource::request) starts the
/// work and returns; the answer arrives later through
/// [`MessageList::deliver`].
pub trait PageSource {
    /// How many rows the current query matches.
    fn total(&self) -> u32;

    /// Start loading `page`, whose rows are positions
    /// `page * PAGE_SIZE .. (page + 1) * PAGE_SIZE`.
    ///
    /// Called at most once per page while that page is outstanding or cached.
    fn request(&self, page: u32);
}

mod row_imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageRow {
        pub row: RefCell<Option<Row>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageRow {
        const NAME: &'static str = "PostioMessageRow";
        type Type = super::MessageRow;
    }

    impl ObjectImpl for MessageRow {}
}

glib::wrapper! {
    /// One item in the model: a loaded [`Row`], or a placeholder for a
    /// position whose page has not arrived.
    pub struct MessageRow(ObjectSubclass<row_imp::MessageRow>);
}

impl Default for MessageRow {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MessageRow {
    /// A row that is still loading.
    pub fn placeholder() -> Self {
        Self::default()
    }

    /// A loaded row.
    pub fn new(row: Row) -> Self {
        let object = Self::default();
        object.set_row(row);
        object
    }

    /// Whether the page carrying this position has arrived.
    pub fn is_loaded(&self) -> bool {
        self.imp().row.borrow().is_some()
    }

    /// The row's data, if it has arrived.
    pub fn row(&self) -> Option<Row> {
        self.imp().row.borrow().clone()
    }

    /// The message this row stands for, if it has arrived.
    pub fn id(&self) -> Option<MessageId> {
        self.imp().row.borrow().as_ref().map(|row| row.id)
    }

    /// Replace the data in place, keeping the object's identity.
    ///
    /// This is what makes a flag change cheap: the same `GObject` stays where
    /// it is, so nothing above has to rediscover which row the selection is on.
    pub fn set_row(&self, row: Row) {
        *self.imp().row.borrow_mut() = Some(row);
    }
}

impl ListRow for MessageRow {
    fn thread(&self) -> Option<ThreadId> {
        // Both conditions, and they are the ones `commands::aim_at_the_
        // conversation` used to apply by hand: a row that does not stand for
        // a conversation is not one, and a row that does but carries no
        // thread id is not one the verbs can name. `postio_core::aim` reads
        // this through `postio_ui`'s blanket `RowFacts`.
        let row = self.row()?;
        row.is_thread().then_some(row.thread).flatten()
    }

    fn id(&self) -> Option<MessageId> {
        // Not recursion: an inherent method always wins method resolution
        // over a trait method for a concrete receiver, and `Self::id` here
        // has a concrete `MessageRow`. `postio_ui::list::ListWindow<T>`
        // reaches this arm through the trait bound instead, which is the
        // only place it is actually called.
        self.id()
    }

    /// Update the existing `GObject` in place and hand that back, so a
    /// redelivered row does not invalidate anything holding onto it — the
    /// behaviour the default (take the incoming value) is wrong for, and
    /// exactly the thing a second frontend must not have to re-derive.
    fn reconcile(existing: &Self, incoming: Self) -> Self {
        if let Some(row) = incoming.row() {
            existing.set_row(row);
        }
        existing.clone()
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageList {
        pub source: RefCell<Option<Rc<dyn PageSource>>>,
        pub window: RefCell<ListWindow<MessageRow>>,
        /// Whether the model is part-way through answering `item()`.
        ///
        /// See [`super::MessageList::hold`]: anything that would emit
        /// `items_changed` while this is set waits for the next turn of the
        /// main loop instead.
        pub reading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageList {
        const NAME: &'static str = "PostioMessageList";
        type Type = super::MessageList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for MessageList {}

    impl ListModelImpl for MessageList {
        fn item_type(&self) -> glib::Type {
            super::MessageRow::static_type()
        }

        fn n_items(&self) -> u32 {
            self.window.borrow().total()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            // `row_at` may ask the source for a page, and a source that
            // answers before it returns would change the model from inside
            // this call. Marking the read is what lets those changes be
            // held until it is over.
            let outer = self.reading.replace(true);
            let row = self.obj().row_at(position);
            self.reading.set(outer);
            row.map(|row| row.upcast())
        }
    }
}

glib::wrapper! {
    /// A `GListModel` over a mailbox, windowed rather than loaded.
    pub struct MessageList(ObjectSubclass<imp::MessageList>)
        @implements gio::ListModel;
}

impl Default for MessageList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MessageList {
    /// An empty list with no source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the model is part-way through answering `item()`.
    ///
    /// `PageSource::request` is called from inside the model answering
    /// `item()` — that is the whole design, and it is what keeps the fetch
    /// off the read path. What it also means is that a source which delivers
    /// before returning changes the model while a view is part-way through
    /// reading it. `GtkListView` does not survive that: it segfaults, with no
    /// message, a long way from the mistake that caused it.
    ///
    /// The contract already says answers arrive later, and a real source —
    /// a repository call marshalled through the `postio-core` bridge —
    /// cannot answer any sooner. But a test double, a bench or the next view
    /// written in a hurry can, and taking the process down is a
    /// disproportionate punishment for it. So the change is held for one turn
    /// of the main loop, by which time the read is over and it is merely
    /// late.
    fn reading(&self) -> bool {
        self.imp().reading.get()
    }

    /// Re-run `action` on the next turn of the main loop. See [`reading`].
    ///
    /// [`reading`]: Self::reading
    fn hold(&self, action: impl FnOnce(&MessageList) + 'static) {
        let list = self.clone();
        glib::idle_add_local_once(move || action(&list));
    }

    /// The generation currently in force.
    ///
    /// Stamp this on a request when it is made — [`crate::feed`] does, on
    /// every [`PageSource::request`] — and pass it back to
    /// [`deliver_for`](Self::deliver_for) or
    /// [`deliver_page`](Self::deliver_page) when the answer arrives. A reply
    /// carrying an older generation is dropped rather than applied: the
    /// scope changed while it was in flight, and answering the question
    /// nobody is asking any more would fill the new scope with the old
    /// one's mail.
    pub fn generation(&self) -> u64 {
        self.imp().window.borrow().generation()
    }

    /// Point the list at a new query: a different folder, or a search.
    ///
    /// Everything cached is dropped — it answered a different question — and
    /// the generation moves on, so any reply already in flight for the
    /// previous one is now stale.
    pub fn set_source(&self, source: Rc<dyn PageSource>) {
        if self.reading() {
            self.hold(move |list| list.set_source(source));
            return;
        }
        let removed = self.imp().window.borrow().total();
        let total = source.total();

        *self.imp().source.borrow_mut() = Some(source);
        self.imp().window.borrow_mut().reset(total);

        self.items_changed(0, removed, total);
    }

    /// Hand the model a page it asked for, assuming it answers the scope
    /// currently in view.
    ///
    /// For a caller with no generation to compare — a test double, or
    /// [`PageSource::request`] answering synchronously — this always
    /// applies. A caller that captured the generation at request time and
    /// needs a stale answer dropped instead wants
    /// [`deliver_for`](Self::deliver_for).
    ///
    /// Rows already resident for the same message keep their `GObject`
    /// ([`ListRow::reconcile`]), so a redelivered page does not invalidate
    /// anything holding onto them.
    pub fn deliver(&self, page: u32, rows: Vec<Row>) {
        let generation = self.generation();
        self.deliver_for(generation, page, rows);
    }

    /// The same as [`deliver`](Self::deliver), but `generation` is checked
    /// against the one currently in force first.
    pub fn deliver_for(&self, generation: u64, page: u32, rows: Vec<Row>) {
        if self.reading() {
            self.hold(move |list| list.deliver_for(generation, page, rows));
            return;
        }
        let items: Vec<MessageRow> = rows.into_iter().map(MessageRow::new).collect();
        let delivered = self
            .imp()
            .window
            .borrow_mut()
            .deliver(generation, page, items);
        if let Some(range) = delivered.changed {
            let span = range.end - range.start;
            self.items_changed(range.start, span, span);
        }
    }

    /// A page of a *mailbox* arrived: a fresh `total` alongside this page's
    /// `rows`, generation-checked as one decision.
    ///
    /// What [`deliver_for`](Self::deliver_for) is for a result set's page,
    /// which carries no total of its own — a mailbox's answer always does,
    /// and the total is applied first, the same order `set_total` and
    /// `deliver` used to run in, so a page delivered before the list knows
    /// how long it is would not be dropped by the very count it just
    /// supplied.
    pub fn deliver_page(&self, generation: u64, total: u32, page: u32, rows: Vec<Row>) {
        if self.reading() {
            self.hold(move |list| list.deliver_page(generation, total, page, rows));
            return;
        }
        if generation != self.generation() {
            return;
        }
        self.set_total(total);
        self.deliver_for(generation, page, rows);
    }

    /// Correct the row count without touching what is cached.
    ///
    /// For a total that shrank or grew at the *end* of the list — a mailbox
    /// recount. New mail arriving at the top is [`inserted_at_top`].
    ///
    /// [`inserted_at_top`]: Self::inserted_at_top
    pub fn set_total(&self, total: u32) {
        if self.reading() {
            self.hold(move |list| list.set_total(total));
            return;
        }
        // Not `if let Some(..) = ...borrow_mut()...` — a temporary borrowed
        // directly in an `if let` condition lives for the whole statement,
        // so it would still be held when `items_changed` below re-enters
        // `item()` synchronously (a listening `GtkListView` does). Binding it
        // first ends that statement, and the borrow, before the signal fires.
        let change = self.imp().window.borrow_mut().set_total(total);
        if let Some((position, removed, added)) = change {
            self.items_changed(position, removed, added);
        }
    }

    /// New mail landed at the top of the list.
    ///
    /// Every row shifts down by `count`, which misaligns every cached page
    /// against its positions, so the cache is dropped and refetched. That
    /// costs a page request; keeping a stale, misaligned cache would cost
    /// correctness. What it deliberately does *not* do is reset the model:
    /// `items_changed` at position 0 is an insertion, so a selection model
    /// moves the selection down with the row it is on and the view keeps its
    /// scroll anchor.
    pub fn inserted_at_top(&self, count: u32) {
        if self.reading() {
            self.hold(move |list| list.inserted_at_top(count));
            return;
        }
        // See the note in `set_total`: bind before branching, so the borrow
        // is gone before `items_changed` can re-enter `item()`.
        let inserted = self.imp().window.borrow_mut().inserted_at_top(count);
        if inserted {
            self.items_changed(0, 0, count);
        }
    }

    /// A message changed in place: read, flagged, answered.
    ///
    /// Cheap and local — the row keeps its `GObject` and its position, so
    /// nothing reloads and nothing loses its place. Returns whether the row
    /// was resident; a message that is not on screen needs no update, because
    /// its page will be fetched fresh when it is. A call made while the model
    /// is answering `item()` is held (see [`hold`](Self::hold)) and reports
    /// `false`, because by then the answer is not yet knowable.
    pub fn update_row(&self, row: Row) -> bool {
        if self.reading() {
            self.hold(move |list| {
                list.update_row(row);
            });
            // Held, so whether the row was resident is not yet knowable.
            // The caller learns nothing, which is honest — and no caller
            // that goes through `crate::feed` asks.
            return false;
        }
        let incoming = MessageRow::new(row);
        let Some(position) = self.imp().window.borrow_mut().update(incoming) else {
            return false;
        };
        self.items_changed(position, 1, 1);
        true
    }

    /// Drop everything cached and ask again, keeping the row count and the
    /// generation.
    ///
    /// The blunt instrument, for when the order itself changed but the
    /// question being answered has not — a request already in flight from
    /// before this call is still answering it, so it is not stale.
    pub fn invalidate(&self) {
        if self.reading() {
            self.hold(|list| list.invalidate());
            return;
        }
        let total = self.imp().window.borrow_mut().invalidate();
        self.items_changed(0, total, total);
    }

    /// How many rows are resident. The number the memory budget is about.
    pub fn resident_rows(&self) -> usize {
        self.imp().window.borrow().resident_rows()
    }

    /// Which resident page holds `message`, if any.
    ///
    /// The cheap half of reacting to a change: a message that changed
    /// somewhere off screen needs nothing done, and one that is on screen
    /// costs a refetch of its page rather than of the folder.
    pub fn page_of(&self, message: MessageId) -> Option<u32> {
        self.imp().window.borrow().page_of(message)
    }

    /// Every resident page holding any of `messages`, deduplicated.
    ///
    /// The bulk form of [`page_of`](Self::page_of), for a burst of changes
    /// that land together, so the caller issues one request per affected
    /// page rather than one per message.
    pub fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        self.imp().window.borrow().pages_holding(messages)
    }

    /// Where `message` sits, among the pages currently resident.
    ///
    /// `None` covers both "not in this mailbox" and "resident but not
    /// fetched yet" — a caller that wants to put the cursor on a message it
    /// did not just deliver itself (a notification's click, landing on
    /// whatever page happens to already be cached) cannot tell those apart
    /// and has to treat them the same: ask for the page, and try again once
    /// it answers.
    pub fn position_of(&self, message: MessageId) -> Option<u32> {
        self.imp().window.borrow().position_of(message)
    }

    /// Which pages are resident, lowest first. For tests and diagnostics.
    pub fn resident_pages(&self) -> Vec<u32> {
        self.imp().window.borrow().resident_pages()
    }

    /// The message at `position`, but only if its page is already here.
    ///
    /// [`row_at`](Self::row_at) fetches what it does not have, which is
    /// right for drawing — a row on screen has to become real — and wrong for
    /// answering "what is in this range". A Shift-click across ten thousand
    /// rows must not ask the store for ten thousand rows: the ones the user
    /// scrolled through are resident and get selected, and the ones they
    /// jumped over were never on screen. Selecting those is what `Ctrl+A`
    /// is for, and it does it with a predicate rather than a list.
    pub fn peek(&self, position: u32) -> Option<MessageId> {
        self.imp().window.borrow().peek(position)
    }

    /// The row at `position`, fetching its page if it is not resident.
    fn row_at(&self, position: u32) -> Option<MessageRow> {
        let mut window = self.imp().window.borrow_mut();
        let lookup = window.row_at(position)?;
        match lookup {
            Lookup::Resident(row) => Some(row.clone()),
            Lookup::Missing { request } => {
                drop(window);
                for page in request {
                    self.request(page);
                }
                Some(MessageRow::placeholder())
            }
        }
    }

    /// Ask the source for `page`.
    ///
    /// Called only for a page [`ListWindow`] has already confirmed needs a
    /// fresh request — deduplication against what is cached or already
    /// pending happens there, not here.
    fn request(&self, page: u32) {
        let source = self.imp().source.borrow().clone();
        if let Some(source) = source {
            source.request(page);
        }
    }
}

/// The list model is what `postio_core::aim` asks about rows.
///
/// Delegating to the `ListWindow` inside rather than reimplementing the rule:
/// `postio_ui`'s blanket implementation is the shared answer, and the FFI
/// boundary reaches the same one through the same window. All this adds is
/// the borrow, taken and released per call so nothing holds it across a
/// callback that might want the window itself.
impl postio_core::aim::RowFacts for MessageList {
    fn row_kind(&self, message: MessageId) -> postio_core::aim::RowKind {
        self.imp().window.borrow().row_kind(message)
    }
}
