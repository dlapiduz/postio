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
//! # What it costs
//!
//! * [`CACHE_PAGES`] pages resident, evicted least-recently-used. Scrolling
//!   the length of a huge folder does not grow that number.
//! * One request per page, ever, until it is evicted — [`PageSource::request`]
//!   is never called twice for a page that is already on its way.
//! * A page either side of the one being read, prefetched, so scrolling at
//!   speed does not stutter on a page boundary.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// Rows per page.
///
/// Big enough that a screenful never spans more than two pages at any sensible
/// density, small enough that a page is a few kilobytes on the wire from
/// SQLite.
pub const PAGE_SIZE: u32 = 50;

/// Pages kept in memory. Everything past this is evicted least-recently-used.
///
/// Eight pages is around 400 rows: roughly a screen either side of the
/// viewport at the airiest density, plus slack for a fast flick.
pub const CACHE_PAGES: usize = 8;

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

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageList {
        pub source: RefCell<Option<Rc<dyn PageSource>>>,
        pub total: Cell<u32>,
        /// Loaded pages, by page index.
        pub pages: RefCell<HashMap<u32, Vec<MessageRow>>>,
        /// Page indices, least recently used first.
        pub recent: RefCell<VecDeque<u32>>,
        /// Pages already asked for and not yet delivered.
        pub pending: RefCell<HashSet<u32>>,
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
            self.total.get()
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

    /// Point the list at a new query: a different folder, or a search.
    ///
    /// Everything cached is dropped — it answered a different question.
    pub fn set_source(&self, source: Rc<dyn PageSource>) {
        if self.reading() {
            self.hold(move |list| list.set_source(source));
            return;
        }
        let imp = self.imp();
        let removed = imp.total.get();
        let total = source.total();

        *imp.source.borrow_mut() = Some(source);
        imp.pages.borrow_mut().clear();
        imp.recent.borrow_mut().clear();
        imp.pending.borrow_mut().clear();
        imp.total.set(total);

        self.items_changed(0, removed, total);
    }

    /// Hand the model a page it asked for.
    ///
    /// Rows already resident for the same message keep their `GObject`, so a
    /// redelivered page does not invalidate anything holding onto them.
    pub fn deliver(&self, page: u32, rows: Vec<Row>) {
        if self.reading() {
            self.hold(move |list| list.deliver(page, rows));
            return;
        }
        let imp = self.imp();
        imp.pending.borrow_mut().remove(&page);

        let existing: HashMap<MessageId, MessageRow> = imp
            .pages
            .borrow()
            .get(&page)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| Some((row.id()?, row.clone())))
                    .collect()
            })
            .unwrap_or_default();

        let count = rows.len() as u32;
        let items: Vec<MessageRow> = rows
            .into_iter()
            .map(|row| match existing.get(&row.id) {
                Some(item) => {
                    item.set_row(row);
                    item.clone()
                }
                None => MessageRow::new(row),
            })
            .collect();

        imp.pages.borrow_mut().insert(page, items);
        self.touch(page);
        self.evict();

        // The positions did not move; what they answer with did.
        let start = page * PAGE_SIZE;
        let span = count.min(imp.total.get().saturating_sub(start));
        if span > 0 {
            self.items_changed(start, span, span);
        }
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
        let imp = self.imp();
        let previous = imp.total.get();
        if previous == total {
            return;
        }
        imp.total.set(total);

        if total > previous {
            self.items_changed(previous, 0, total - previous);
        } else {
            self.drop_pages_from(total);
            self.items_changed(total, previous - total, 0);
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
        if count == 0 {
            return;
        }
        let imp = self.imp();
        imp.pages.borrow_mut().clear();
        imp.recent.borrow_mut().clear();
        imp.pending.borrow_mut().clear();
        imp.total.set(imp.total.get() + count);
        self.items_changed(0, 0, count);
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
        let imp = self.imp();
        let found = imp.pages.borrow().iter().find_map(|(page, items)| {
            let index = items.iter().position(|item| item.id() == Some(row.id))?;
            Some((*page, index, items[index].clone()))
        });

        let Some((page, index, item)) = found else {
            return false;
        };
        item.set_row(row);
        let position = page * PAGE_SIZE + index as u32;
        self.items_changed(position, 1, 1);
        true
    }

    /// Drop everything cached and ask again, keeping the row count.
    ///
    /// The blunt instrument, for when the order itself changed.
    pub fn invalidate(&self) {
        if self.reading() {
            self.hold(|list| list.invalidate());
            return;
        }
        let imp = self.imp();
        imp.pages.borrow_mut().clear();
        imp.recent.borrow_mut().clear();
        imp.pending.borrow_mut().clear();
        let total = imp.total.get();
        self.items_changed(0, total, total);
    }

    /// How many rows are resident. The number the memory budget is about.
    pub fn resident_rows(&self) -> usize {
        self.imp().pages.borrow().values().map(Vec::len).sum()
    }

    /// Which resident page holds `message`, if any.
    ///
    /// The cheap half of reacting to a change: a message that changed
    /// somewhere off screen needs nothing done, and one that is on screen
    /// costs a refetch of its page rather than of the folder.
    pub fn page_of(&self, message: MessageId) -> Option<u32> {
        self.position_of(message)
            .map(|position| position / PAGE_SIZE)
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
        self.imp().pages.borrow().iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.id() == Some(message))
                .map(|index| page * PAGE_SIZE + index as u32)
        })
    }

    /// Which pages are resident, lowest first. For tests and diagnostics.
    pub fn resident_pages(&self) -> Vec<u32> {
        let mut pages: Vec<u32> = self.imp().pages.borrow().keys().copied().collect();
        pages.sort_unstable();
        pages
    }

    /// The message at `position`, but only if its page is already here.
    ///
    /// [`row_at`](Self::row_at) fetches what it does not have, which is right
    /// for drawing — a row on screen has to become real — and wrong for
    /// answering "what is in this range". A Shift-click across ten thousand
    /// rows must not ask the store for ten thousand rows: the ones the user
    /// scrolled through are resident and get selected, and the ones they
    /// jumped over were never on screen. Selecting those is what `Ctrl+A`
    /// is for, and it does it with a predicate rather than a list.
    pub fn peek(&self, position: u32) -> Option<MessageId> {
        let imp = self.imp();
        if position >= imp.total.get() {
            return None;
        }
        imp.pages
            .borrow()
            .get(&(position / PAGE_SIZE))
            .and_then(|rows| rows.get((position % PAGE_SIZE) as usize))
            .and_then(MessageRow::id)
    }

    /// The row at `position`, fetching its page if it is not resident.
    fn row_at(&self, position: u32) -> Option<MessageRow> {
        let imp = self.imp();
        if position >= imp.total.get() {
            return None;
        }

        let page = position / PAGE_SIZE;
        let index = (position % PAGE_SIZE) as usize;

        let cached = imp
            .pages
            .borrow()
            .get(&page)
            .and_then(|rows| rows.get(index))
            .cloned();
        if let Some(row) = cached {
            self.touch(page);
            return Some(row);
        }

        // Not here: ask for it, and for the pages either side, so scrolling at
        // speed does not stall on a page boundary.
        self.request(page);
        if page > 0 {
            self.request(page - 1);
        }
        self.request(page + 1);

        Some(MessageRow::placeholder())
    }

    /// Ask the source for `page`, unless it is already cached or on its way.
    fn request(&self, page: u32) {
        let imp = self.imp();
        if page * PAGE_SIZE >= imp.total.get() {
            return;
        }
        if imp.pages.borrow().contains_key(&page) || !imp.pending.borrow_mut().insert(page) {
            return;
        }
        let source = imp.source.borrow().clone();
        if let Some(source) = source {
            source.request(page);
        }
    }

    /// Mark `page` as the most recently used.
    fn touch(&self, page: u32) {
        let mut recent = self.imp().recent.borrow_mut();
        recent.retain(|p| *p != page);
        recent.push_back(page);
    }

    /// Drop the least recently used pages down to [`CACHE_PAGES`].
    ///
    /// No `items_changed` for what goes: an evicted page is by definition the
    /// one nobody has looked at in longest, so nothing is bound to it. If the
    /// view does come back, it gets a placeholder and a fresh request.
    fn evict(&self) {
        let imp = self.imp();
        while imp.recent.borrow().len() > CACHE_PAGES {
            let Some(oldest) = imp.recent.borrow_mut().pop_front() else {
                break;
            };
            imp.pages.borrow_mut().remove(&oldest);
        }
    }

    /// Forget every cached page that lies wholly past `position`.
    fn drop_pages_from(&self, position: u32) {
        let imp = self.imp();
        let first_stale = position.div_ceil(PAGE_SIZE);
        imp.pages.borrow_mut().retain(|page, _| *page < first_stale);
        imp.recent.borrow_mut().retain(|page| *page < first_stale);
        imp.pending.borrow_mut().retain(|page| *page < first_stale);
    }
}
