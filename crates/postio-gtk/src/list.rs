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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::list::{ListRow, ListWindow, Lookup, Splice};

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
    /// The thread it belongs to, for the conversation pane and the count badge.
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
    /// Whether it is a draft, and which state its send is in.
    ///
    /// The renderer needs the state, not the fact: a queued send and a failed
    /// one are both "a draft", and drawing them the same is the defect the
    /// Outbox exists to fix (#1491).
    pub send_state: Option<postio_model::DraftState>,
    /// When a scheduled send is due, so the row can say *when* rather than
    /// just that it is waiting (spec 003 FR-007).
    pub send_at: Option<chrono::DateTime<chrono::Utc>>,
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

    impl ObjectImpl for MessageRow {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                // What this row says has changed, while the row itself stayed
                // where it is. A bound view re-reads and redraws; the model
                // stays quiet. See `MessageRow::set_row`.
                vec![glib::subclass::Signal::builder("changed").build()]
            })
        }
    }
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
    /// it is, so nothing above has to rediscover which row the selection is on
    /// — and, since #1216, nothing above has to be told through the *model*
    /// either. A row emits `changed` for itself, which a bound view answers by
    /// redrawing one row. Saying it through `items_changed` instead is how
    /// reading one message came to repaint the whole visible list: that signal
    /// can only mean "these positions answer with different rows now", and
    /// `GtkListView` answers it by rebuilding every widget in range.
    ///
    /// Quiet when nothing actually moved, so a page redelivered unchanged —
    /// which a resync does constantly — costs no redraw at all.
    pub fn set_row(&self, row: Row) {
        {
            let mut held = self.imp().row.borrow_mut();
            if held.as_ref() == Some(&row) {
                return;
            }
            *held = Some(row);
        }
        self.emit_by_name::<()>("changed", &[]);
    }

    /// Call `on_change` whenever this row's contents are replaced.
    ///
    /// The handler id is the caller's to disconnect: a `GtkListItem` is
    /// recycled across many rows, and a connection left behind would redraw a
    /// widget for a message it is no longer showing.
    pub fn connect_changed(&self, on_change: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("changed", false, move |values| {
            if let Some(row) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_change(&row);
            }
            None
        })
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
        /// The `MessageRow` each position answers with, for as long as that
        /// position means the same thing.
        ///
        /// A `GListModel` says "this position holds a different row now" with
        /// `items_changed`, and `GtkListView` answers it by building a widget
        /// for every item in range — measured, on a list whose viewport holds
        /// ten rows: a page delivery of fifty built fifty (#1216). A page
        /// landing on positions that already exist is not that change. It is
        /// the same positions, holding the same messages, saying something
        /// they could not say yet, and the way to say *that* is to fill in the
        /// object the view is already holding and let it announce itself.
        ///
        /// Which only works if there is one object per position to fill. Handing
        /// out a fresh placeholder per `item()` call left the view holding
        /// objects the model had no way to reach.
        ///
        /// Dropped when a position stops meaning what it did — a new scope, an
        /// insertion at the top, an eviction — because an object kept across
        /// that would answer for the wrong message.
        pub handed: RefCell<HashMap<u32, super::MessageRow>>,
        /// A refresh whose pages are still on their way. See
        /// [`super::MessageList::refresh`].
        pub refresh: RefCell<Option<super::Refresh>>,
        /// Whether a refresh is part-way through telling the view what
        /// moved. A position asked for in between answers without asking
        /// the source: the step after this one may well fill it.
        pub splicing: Cell<bool>,
        /// The source the list is about to show, and the generation its
        /// first page will be answered under. See
        /// [`super::MessageList::replace_source`].
        pub next: RefCell<Option<super::Next>>,
        /// The mailbox a result set took the list from, kept whole so `Esc`
        /// can put it back without reading it again.
        pub stashed: RefCell<Option<ListWindow<super::MessageRow>>>,
        /// Whether the list has not yet heard from the source it was
        /// pointed at. Not the same as empty: see
        /// [`super::MessageList::is_loading`].
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageList {
        const NAME: &'static str = "PostioMessageList";
        type Type = super::MessageList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for MessageList {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                // Rows that already existed now have contents. Not
                // `items_changed`: nothing moved, nothing was replaced, and
                // saying it that way costs a rebuilt widget per row in range.
                // What it is for is everyone who is not a `GtkListView` --
                // the reading pane following the cursor onto a row that has
                // just become real, and a seek waiting for the page carrying
                // the message it is looking for.
                vec![
                    glib::subclass::Signal::builder("filled").build(),
                    // A refresh is about to tell the view what moved (`true`),
                    // or has finished (`false`). Between the two a row can be
                    // taken out and put back somewhere else, and a cursor
                    // that followed the position would be on another
                    // message -- so whoever keeps the cursor holds it still
                    // across the pair.
                    glib::subclass::Signal::builder("splicing")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }
    }

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

/// How many `items_changed` this process has emitted from a message list.
///
/// A diagnostic in the counting idiom `postio_storage::test_support::counting`
/// uses for SQLite: a number that is the same on every machine, where the
/// duration it causes is not. `GtkListView` answers each emission by
/// re-examining the model and rebuilding the widgets it tracks, so what a
/// navigation costs is roughly linear in this — which is why it is worth
/// counting rather than timing (#1216).
pub fn emissions() -> u64 {
    EMISSIONS.load(std::sync::atomic::Ordering::Relaxed)
}

static EMISSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The source a list is about to show. See [`MessageList::replace_source`].
pub struct Next {
    source: Rc<dyn PageSource>,
    generation: u64,
    /// Keep the rows being replaced, for [`MessageList::restore`].
    stash: bool,
    /// A refresh was asked for while this was on its way.
    refresh: bool,
}

/// A refresh in flight: the pages it re-read, and what has come back.
///
/// See [`MessageList::refresh`].
#[derive(Debug)]
pub struct Refresh {
    generation: u64,
    /// Runs of pages, first and last inclusive, each re-read whole.
    runs: Vec<(u32, u32)>,
    awaiting: HashSet<u32>,
    arrived: HashMap<u32, Vec<Row>>,
    total: Option<u32>,
    /// Another refresh was asked for while this one was out; run it once
    /// this one lands rather than piling a second read on top.
    again: bool,
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
    /// Every row the list holds, in no particular order, asking the source
    /// for nothing. See [`postio_ui::list::ListWindow::resident`].
    pub fn held_rows(&self) -> Vec<Row> {
        self.imp()
            .window
            .borrow()
            .resident()
            .filter_map(|entry| entry.row())
            .collect()
    }

    /// Call `on_filled` when a delivery gives contents to rows that already
    /// existed.
    ///
    /// For everyone who is not a `GtkListView`: the reading pane follows the
    /// cursor onto a row that has just become real, and a seek waits for the
    /// page carrying the message it wants. Both used to ride on
    /// `items_changed`, which a page delivery no longer emits (#1216).
    pub fn connect_filled(&self, on_filled: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("filled", false, move |values| {
            if let Some(list) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_filled(&list);
            }
            None
        })
    }

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
    /// Above GDK's redraw band, deliberately, and that is not a detail.
    /// `idle_add_local_once` runs at `G_PRIORITY_DEFAULT_IDLE` — 200 — and
    /// GDK paints at `GDK_PRIORITY_REDRAW`, which is `G_PRIORITY_HIGH_IDLE`
    /// plus 20, so 120. A window with something to paint every time the loop
    /// looks therefore outranks the held change for as long as the painting
    /// lasts, and "the next turn of the main loop" quietly becomes "some turn
    /// after the window stops being busy".
    ///
    /// Under load that is not a frame or two. #1015 caught a 300-message list
    /// sitting at nought resident rows for a full five seconds, mapped and
    /// painting the whole time — a list that stays blank while its window is
    /// busy, which is exactly the shape of bug the hold exists to avoid
    /// causing. A deferral a repaint can postpone indefinitely is not a
    /// deferral.
    ///
    /// `HIGH_IDLE` is also the right side of the line for what this is *for*:
    /// the change has to land before the frame that would otherwise draw the
    /// state it corrects.
    ///
    /// [`reading`]: Self::reading
    fn hold(&self, action: impl FnOnce(&MessageList) + 'static) {
        let list = self.clone();
        // `_full` rather than `_once` because only the former takes a
        // priority; the `Option` is what makes an `FnOnce` fit its `FnMut`.
        let mut action = Some(action);
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            if let Some(action) = action.take() {
                action(&list);
            }
            glib::ControlFlow::Break
        });
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
        if let Some(next) = self.imp().next.borrow().as_ref() {
            return next.generation;
        }
        self.imp().window.borrow().generation()
    }

    /// Whether the list has not yet heard from the source it was pointed at.
    ///
    /// Not the same as empty, and the difference is what a person sees: an
    /// empty folder says so, and a folder whose first page is still on its
    /// way must not say it for the frame or two before the page lands.
    pub fn is_loading(&self) -> bool {
        self.imp().loading.get()
    }

    /// Call `on_splicing` with `true` before a refresh tells the view what
    /// moved, and with `false` once it has.
    pub fn connect_splicing(
        &self,
        on_splicing: impl Fn(&Self, bool) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_local("splicing", false, move |values| {
            let list = values.first().and_then(|value| value.get::<Self>().ok());
            let begun = values.get(1).and_then(|value| value.get::<bool>().ok());
            if let (Some(list), Some(begun)) = (list, begun) {
                on_splicing(&list, begun);
            }
            None
        })
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
        let generation = self.generation() + 1;

        self.imp().next.replace(None);
        self.imp().refresh.replace(None);
        self.imp().loading.set(true);
        *self.imp().source.borrow_mut() = Some(source);
        self.imp().window.borrow_mut().adopt(total, generation);
        // Every position now stands for a different mailbox's mail.
        self.imp().handed.borrow_mut().clear();

        EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.items_changed(0, removed, total);
    }

    /// Point the list at a new query, but keep showing the rows it has until
    /// the new one's first page lands, then change over in one step.
    ///
    /// [`set_source`](Self::set_source) empties the list at once, and the
    /// frame or two before the first page arrives drew every visible row as
    /// a skeleton -- and, in a folder switch, told the list-state pane there
    /// was no mail, so it said "Inbox is empty" on its way to showing the
    /// inbox. Keeping the old rows for those milliseconds is what a person
    /// reads as instant. The change-over is one `items_changed` with the
    /// first page already resident, so every row the view asks for then is
    /// real.
    ///
    /// [`generation`](Self::generation) moves on now, so the first page is
    /// requested and answered under the new question; a page of the old one
    /// still in flight is dropped. The old rows ask for nothing while they
    /// wait. A list showing nothing has nothing to keep, and changes over at
    /// once.
    ///
    /// `stash` keeps the rows being replaced, for [`restore`](Self::restore)
    /// -- a mailbox a search is about to cover. Only the first stash
    /// counts: refining the query replaces one result set with another, and
    /// the mailbox underneath is still the one to go back to.
    pub fn replace_source(&self, source: Rc<dyn PageSource>, stash: bool) {
        if self.reading() {
            self.hold(move |list| list.replace_source(source, stash));
            return;
        }
        let imp = self.imp();
        imp.refresh.replace(None);
        if !stash {
            imp.stashed.replace(None);
        }
        let showing = {
            let window = imp.window.borrow();
            window.total() > 0 && !window.resident_pages().is_empty()
        };
        if !showing {
            self.set_source(source);
            return;
        }
        let generation = self.generation() + 1;
        imp.next.replace(Some(Next {
            source,
            generation,
            stash,
            refresh: false,
        }));
        imp.loading.set(true);
    }

    /// Put back the mailbox a result set covered, rows and all, and return
    /// whether there was one.
    ///
    /// No read is needed to show it: its pages are the ones it had, under a
    /// new generation so whatever the result set still has in flight is
    /// dropped. The caller refreshes it afterwards, which re-reads only what
    /// is on screen and moves only what changed while the search was up.
    /// A result set still waiting for its first page never took the list,
    /// so the mailbox is simply still there.
    pub fn restore(&self, source: Rc<dyn PageSource>) -> bool {
        if self.reading() {
            return false;
        }
        let imp = self.imp();
        if imp.next.borrow().is_some() && imp.stashed.borrow().is_none() {
            // Nothing was swapped out; drop the swap that was coming.
            let generation = self.generation() + 1;
            imp.next.replace(None);
            imp.loading.set(false);
            *imp.source.borrow_mut() = Some(source);
            // A new generation for the rows already here, so the result
            // set's first page, still out, cannot land on them.
            imp.window.borrow_mut().renumber(generation);
            return true;
        }
        let Some(mut kept) = imp.stashed.take() else {
            return false;
        };
        let generation = self.generation() + 1;
        let total = kept.total();
        kept.renumber(generation);
        let removed = imp.window.borrow().total();
        imp.next.replace(None);
        imp.refresh.replace(None);
        imp.loading.set(false);
        *imp.source.borrow_mut() = Some(source);
        *imp.window.borrow_mut() = kept;
        imp.handed.borrow_mut().clear();
        self.swap_in(removed, total);
        true
    }

    /// Change over to the source [`replace_source`](Self::replace_source)
    /// was waiting on, now that a page of it has landed.
    fn change_over(&self, total: u32, page: u32, rows: Vec<Row>) {
        let imp = self.imp();
        let Some(next) = imp.next.take() else {
            return;
        };
        let total = total.max(rows.len() as u32 + page * PAGE_SIZE);
        let removed = imp.window.borrow().total();
        *imp.source.borrow_mut() = Some(next.source);
        imp.loading.set(false);
        {
            let mut fresh = ListWindow::new();
            fresh.adopt(total, next.generation);
            fresh.deliver(
                next.generation,
                page,
                rows.into_iter().map(MessageRow::new).collect(),
            );
            let old = std::mem::replace(&mut *imp.window.borrow_mut(), fresh);
            if next.stash && imp.stashed.borrow().is_none() {
                imp.stashed.replace(Some(old));
            }
        }
        imp.handed.borrow_mut().clear();
        self.swap_in(removed, total);
        if next.refresh {
            self.refresh();
        }
    }

    /// Tell the view that every one of its `removed` rows is now one of a
    /// different list's `total`, which the window already holds.
    ///
    /// In two steps rather than one, and the split is what keeps a folder
    /// switch cheap. `GtkSingleSelection`, told its selected row was
    /// replaced, looks for the item it had among *every* row added -- one
    /// `item()` each -- and every one of those asks for its page: opening a
    /// 3,618-conversation folder asked for 73. So the replace covers only the
    /// rows the window already holds from the top, and the rest arrive as an
    /// insertion at the end, which the selection has no reason to search.
    fn swap_in(&self, removed: u32, total: u32) {
        let head = {
            let mut window = self.imp().window.borrow_mut();
            let head = (0..total)
                .take_while(|position| window.peek(*position).is_some())
                .count() as u32;
            window.set_total(head);
            head
        };
        EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.items_changed(0, removed, head);
        self.set_total(total);
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
        let waiting = self
            .imp()
            .next
            .borrow()
            .as_ref()
            .map(|next| (next.generation, next.source.total()));
        if let Some((next, total)) = waiting {
            if generation == next {
                self.change_over(total, page, rows);
            }
            return;
        }
        let Some(rows) = self.take_for_refresh(generation, page, rows, None) else {
            return;
        };
        // Fill the objects the view is already holding for these positions.
        // Collected before any of them is told, because a handler is free to
        // ask the model for a row and `row_at` takes this borrow mutably.
        let start = page * postio_ui::list::PAGE_SIZE;
        let filling: Vec<(MessageRow, Row)> = {
            let handed = self.imp().handed.borrow();
            rows.iter()
                .enumerate()
                .filter_map(|(offset, row)| {
                    let position = start + offset as u32;
                    handed
                        .get(&position)
                        .map(|held| (held.clone(), row.clone()))
                })
                .collect()
        };

        let items: Vec<MessageRow> = rows.into_iter().map(MessageRow::new).collect();
        let delivered = self
            .imp()
            .window
            .borrow_mut()
            .deliver(generation, page, items);
        if delivered.stale {
            return;
        }
        // The first answer ends the wait even when it brought no rows: an
        // empty folder is only known to be empty once it has said so, and
        // whoever draws that has to hear about it.
        let answered = self.imp().loading.replace(false);
        let filled = !filling.is_empty() || answered;
        for (held, row) in filling {
            held.set_row(row);
        }
        // An evicted page's positions are not on screen -- that is what makes
        // them evictable -- so the objects standing for them can go too, and
        // must, or a list scrolled through a mailbox would keep one per row it
        // passed.
        if !delivered.evicted.is_empty() {
            let mut handed = self.imp().handed.borrow_mut();
            for page in &delivered.evicted {
                let first = page * postio_ui::list::PAGE_SIZE;
                for position in first..first + postio_ui::list::PAGE_SIZE {
                    handed.remove(&position);
                }
            }
        }
        if filled {
            self.emit_by_name::<()>("filled", &[]);
        }
        // Nothing structural happened: the list is the same length, the same
        // positions hold the same messages, and every row the view holds for
        // them has just been told what it now says. Announcing it through the
        // model as well would tell `GtkListView` that a page of positions
        // answers with different rows now, and it would rebuild a widget for
        // every row in range — the whole visible list blinking because one
        // message was read, and half a folder switch's widgets rebuilding rows
        // that had just been built.
        let _ = delivered.changed;
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
        let waiting = self
            .imp()
            .next
            .borrow()
            .as_ref()
            .map(|next| next.generation);
        if let Some(next) = waiting {
            if generation == next {
                self.change_over(total, page, rows);
            }
            return;
        }
        if generation != self.generation() {
            return;
        }
        let Some(rows) = self.take_for_refresh(generation, page, rows, Some(total)) else {
            return;
        };
        self.set_total(total);
        self.deliver_for(generation, page, rows);
    }

    /// Give up on a page whose fetch failed, so the view can ask again.
    ///
    /// The banner an error raises tells a person something went wrong; it
    /// does not put the rows back. Without this the page stays outstanding
    /// for the life of the window and its fifty positions draw skeletons that
    /// nothing can clear -- which is what a live inbox did, almost in its
    /// entirety, while the body backfill ran.
    ///
    /// Behind the same `reading` guard as [`deliver_for`](Self::deliver_for):
    /// `GListModel::item()` must not be re-entered, and a failure can arrive
    /// mid-call exactly as a delivery can.
    pub fn abandon_page(&self, generation: u64, page: u32) {
        if self.reading() {
            self.hold(move |list| list.abandon_page(generation, page));
            return;
        }
        self.imp().window.borrow_mut().abandon(generation, page);
    }

    /// Stop waiting for `page`: its read failed and will not be tried again.
    ///
    /// A refresh that was holding its other pages for this one applies what
    /// it has, and a change-over waiting on a first page that is never
    /// coming changes over to the count the source knows, rows to follow
    /// when they are asked for -- rather than showing the folder it left
    /// for the rest of the session.
    pub fn give_up(&self, generation: u64, page: u32) {
        if self.reading() {
            self.hold(move |list| list.give_up(generation, page));
            return;
        }
        let waiting = self
            .imp()
            .next
            .borrow()
            .as_ref()
            .map(|next| (next.generation, next.source.total()));
        if let Some((next, total)) = waiting {
            if generation == next {
                self.change_over(total, page, Vec::new());
                self.imp().window.borrow_mut().abandon(generation, page);
            }
            return;
        }
        self.imp().window.borrow_mut().abandon(generation, page);
        // Given up on under the question in force: the answer is that there
        // is nothing, which ends the wait exactly as a first page that
        // brought no rows does in `deliver_for`. A search with no hits gets
        // here -- it has no page 0 to ask for -- and without this a list that
        // was empty when it arrived stayed "loading", so the pane withheld
        // "nothing matched" for good.
        if generation == self.generation() && self.imp().loading.replace(false) {
            self.emit_by_name::<()>("filled", &[]);
        }
        let complete = {
            let mut slot = self.imp().refresh.borrow_mut();
            match slot.as_mut() {
                Some(refresh) if refresh.generation == generation => {
                    refresh.awaiting.remove(&page) && refresh.awaiting.is_empty()
                }
                _ => false,
            }
        };
        if complete {
            self.apply_refresh();
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
        // Not `if let Some(..) = ...borrow_mut()...` — a temporary borrowed
        // directly in an `if let` condition lives for the whole statement,
        // so it would still be held when `items_changed` below re-enters
        // `item()` synchronously (a listening `GtkListView` does). Binding it
        // first ends that statement, and the borrow, before the signal fires.
        let change = self.imp().window.borrow_mut().set_total(total);
        // A recount moves the *end* of the list. Positions before it still
        // mean what they did, so their objects stay; the ones past the new end
        // do not exist any more.
        self.imp()
            .handed
            .borrow_mut()
            .retain(|position, _| *position < total);
        if let Some((position, removed, added)) = change {
            EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        self.imp().refresh.replace(None);
        let inserted = self.imp().window.borrow_mut().inserted_at_top(count);
        if inserted {
            // Every row shifted down by `count`, so every position stands for
            // a different message than the object held for it does.
            self.imp().handed.borrow_mut().clear();
            EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        let incoming = MessageRow::new(row.clone());
        let Some(position) = self.imp().window.borrow_mut().update(incoming) else {
            return false;
        };
        // Same position, same message, new contents — so the row says so for
        // itself and the model stays quiet. Only a row nothing is holding for
        // needs telling through `items_changed`, and there is nothing to tell.
        let held = self.imp().handed.borrow().get(&position).cloned();
        match held {
            Some(held) => held.set_row(row),
            None => {
                EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.items_changed(position, 1, 1);
            }
        }
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
        self.imp().refresh.replace(None);
        let total = self.imp().window.borrow_mut().invalidate();
        // The order is what changed, so a position no longer means what it did.
        self.imp().handed.borrow_mut().clear();
        EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.items_changed(0, total, total);
    }

    /// Re-read what is on screen and tell the view only what moved.
    ///
    /// What a resync, a backfill batch, an archive in an aggregate view or
    /// new mail in a unified one all come to: the membership or the order
    /// may have moved, and the list does not know how. It used to answer
    /// with [`invalidate`](Self::invalidate) -- every page dropped, every
    /// position announced as replaced -- and every row on screen flashed to
    /// a skeleton and was built again, 85-121ms before anything painted, on
    /// every batch of a sync (maintainer, 2026-09-25: "full reloads make the
    /// app feel slow").
    ///
    /// Now the rows stay. The pages somebody is looking at -- any page
    /// holding a row the view or the selection still holds -- are read
    /// again, and until every one of them has landed the list goes on
    /// answering with what it had. Then each run of them is compared with
    /// what came back, by message id ([`postio_ui::list::splices`]): a row
    /// still there in the same order stays where it is and is told what it
    /// says now, and everything else is a removal or an insertion *at its
    /// position*, so the view keeps its scroll anchor and every other row's
    /// widget. Nothing moved is nothing emitted. Pages nobody is looking at
    /// are dropped, to be read at their new offsets when they are wanted.
    ///
    /// One refresh at a time: asked for again while one is out, it runs once
    /// more after it lands, however many times it was asked. The count comes
    /// back with the pages and moves the end of the list.
    pub fn refresh(&self) {
        if self.reading() {
            self.hold(|list| list.refresh());
            return;
        }
        let imp = self.imp();
        // A change-over's first page may have been read before whatever
        // this refresh is for; refresh the new rows once they are here.
        if let Some(next) = imp.next.borrow_mut().as_mut() {
            next.refresh = true;
            return;
        }
        if let Some(refresh) = imp.refresh.borrow_mut().as_mut() {
            refresh.again = true;
            return;
        }
        let (generation, resident) = {
            let window = imp.window.borrow();
            (window.generation(), window.resident_pages())
        };
        let mut watched: Vec<u32> = imp
            .handed
            .borrow()
            .iter()
            // One reference is the map's own. Anything more is the view's
            // row for that position, or the selection's.
            .filter(|(_, row)| row.ref_count() > 1)
            .map(|(position, _)| position / PAGE_SIZE)
            .collect();
        if watched.is_empty() {
            watched.push(0);
        }
        watched.sort_unstable();
        watched.dedup();
        watched.retain(|page| resident.contains(page));
        if watched.is_empty() {
            // Nothing on screen is held, so there is nothing to keep and
            // nothing to move. Drop what is cached and read the top, which
            // carries the count, the way a first read does.
            let pending = {
                let mut window = imp.window.borrow_mut();
                window.retain_pages(|_| false);
                window.is_pending(0)
            };
            imp.handed.borrow_mut().retain(|_, row| row.ref_count() > 1);
            if !pending {
                self.request(0);
            }
            return;
        }
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for page in &watched {
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == *page => *last = *page,
                _ => runs.push((*page, *page)),
            }
        }
        imp.refresh.replace(Some(Refresh {
            generation,
            runs,
            awaiting: watched.iter().copied().collect(),
            arrived: HashMap::new(),
            total: None,
            again: false,
        }));
        for page in watched {
            imp.window.borrow_mut().note_pending(page);
            self.request(page);
        }
    }

    /// Hold a page a refresh is waiting for, applying the refresh once the
    /// last of them is in. Hands `rows` back when they are not a refresh's.
    fn take_for_refresh(
        &self,
        generation: u64,
        page: u32,
        rows: Vec<Row>,
        total: Option<u32>,
    ) -> Option<Vec<Row>> {
        let complete = {
            let mut slot = self.imp().refresh.borrow_mut();
            let Some(refresh) = slot.as_mut() else {
                return Some(rows);
            };
            if refresh.generation != generation || !refresh.awaiting.remove(&page) {
                return Some(rows);
            }
            if self.imp().window.borrow_mut().answered(generation, page) {
                refresh.arrived.insert(page, rows);
            }
            if total.is_some() {
                refresh.total = total;
            }
            refresh.awaiting.is_empty()
        };
        if complete {
            self.apply_refresh();
        }
        None
    }

    /// Tell the view what a refresh found. See [`refresh`](Self::refresh).
    fn apply_refresh(&self) {
        let imp = self.imp();
        let Some(mut refresh) = imp.refresh.take() else {
            return;
        };
        if refresh.generation != self.generation() {
            return;
        }
        let asked = refresh.runs.clone();
        let in_run = move |page: u32| {
            asked
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&page))
        };
        // Pages nobody was looking at are one row out for every row this
        // moves above them, so they go. What the view still holds for them
        // stays: it is the same message either way.
        {
            let in_run = in_run.clone();
            imp.window.borrow_mut().retain_pages(in_run);
        }
        imp.handed
            .borrow_mut()
            .retain(|position, row| in_run(position / PAGE_SIZE) || row.ref_count() > 1);

        imp.splicing.set(true);
        self.emit_by_name::<()>("splicing", &[&true]);
        let mut broken = false;
        // Bottom up, so a run's steps never move a position a later run is
        // still to use.
        for (first, last) in refresh.runs.iter().rev() {
            if !(*first..=*last).all(|page| refresh.arrived.contains_key(&page)) {
                continue;
            }
            let start = first * PAGE_SIZE;
            let incoming: Vec<Row> = (*first..=*last)
                .flat_map(|page| refresh.arrived.remove(&page).unwrap_or_default())
                .collect();
            let held: Vec<Option<MessageId>> = {
                let window = imp.window.borrow();
                (start..(last + 1) * PAGE_SIZE)
                    .map_while(|position| window.peek(position).map(Some))
                    .collect()
            };
            let ids: Vec<MessageId> = incoming.iter().map(|row| row.id).collect();
            for step in postio_ui::list::splices(&held, &ids) {
                let (position, removed, rows) = match step {
                    Splice::Remove { at, count } => (start + at, count, Vec::new()),
                    Splice::Insert { at, count, from } => (
                        start + at,
                        0,
                        incoming[from..from + count as usize]
                            .iter()
                            .cloned()
                            .map(MessageRow::new)
                            .collect(),
                    ),
                };
                let added = rows.len() as u32;
                if !imp.window.borrow_mut().splice(position, removed, rows) {
                    broken = true;
                    break;
                }
                self.shift_handed(position, removed, added);
                EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.items_changed(position, removed, added);
            }
            if broken {
                break;
            }
            // What stayed: the same messages in the same places, told what
            // they say now. Quiet for a row that says what it said.
            for (offset, row) in incoming.into_iter().enumerate() {
                let position = start + offset as u32;
                imp.window.borrow_mut().update(MessageRow::new(row.clone()));
                let held = imp.handed.borrow().get(&position).cloned();
                if let Some(held) = held {
                    held.set_row(row);
                }
            }
        }
        imp.splicing.set(false);

        if broken {
            // A step the window could not take: it and the view no longer
            // agree about a position, and only starting over is honest.
            self.emit_by_name::<()>("splicing", &[&false]);
            self.invalidate();
            self.request(0);
            return;
        }

        // A position the view asked for between two steps was answered with
        // a placeholder; fill the ones a later step made real, and ask for
        // the rest.
        let waiting: Vec<(u32, MessageRow)> = imp
            .handed
            .borrow()
            .iter()
            .filter(|(_, row)| !row.is_loaded())
            .map(|(position, row)| (*position, row.clone()))
            .collect();
        let mut filled = false;
        let mut wanted: Vec<u32> = Vec::new();
        for (position, placeholder) in waiting {
            let lookup = {
                let mut window = imp.window.borrow_mut();
                match window.row_at(position) {
                    Some(Lookup::Resident(row)) => Ok(row.row()),
                    Some(Lookup::Missing { request }) => Err(request),
                    None => Ok(None),
                }
            };
            match lookup {
                Ok(Some(row)) => {
                    placeholder.set_row(row);
                    filled = true;
                }
                Ok(None) => {}
                Err(request) => wanted.extend(request),
            }
        }
        for page in wanted {
            self.request(page);
        }
        if let Some(total) = refresh.total {
            self.set_total(total);
        }
        self.emit_by_name::<()>("splicing", &[&false]);
        if filled {
            self.emit_by_name::<()>("filled", &[]);
        }
        if refresh.again {
            self.refresh();
        }
    }

    /// Re-key the objects handed to the view for a splice at `position`:
    /// the `removed` there go, and every one after moves by the difference.
    fn shift_handed(&self, position: u32, removed: u32, added: u32) {
        let mut handed = self.imp().handed.borrow_mut();
        let before = std::mem::take(&mut *handed);
        for (held, row) in before {
            if held < position {
                handed.insert(held, row);
            } else if held >= position + removed {
                handed.insert(held - removed + added, row);
            }
        }
    }

    /// Whether every one of `messages` is a row the list is holding.
    pub fn all_resident(&self, messages: &[MessageId]) -> bool {
        let window = self.imp().window.borrow();
        !messages.is_empty()
            && messages
                .iter()
                .all(|message| window.position_of(*message).is_some())
    }

    /// Take `messages` out of the list where they stand, each row after them
    /// moving up, rather than reloading it (#1607).
    ///
    /// `false`, with nothing changed, unless every one is a resident row
    /// that [`ListWindow::remove_at`](postio_ui::list::ListWindow::remove_at)
    /// can take out: the caller reloads then, as before. Removed from the
    /// bottom up, so no removal moves a position another is still to use,
    /// and announced one row at a time -- `items_changed(position, 1, 0)` --
    /// so the list view keeps every other row's widget, and a cursor on a
    /// removed row lands on the one that moved into its place.
    pub fn remove_in_place(&self, messages: &[MessageId]) -> bool {
        if self.reading() {
            return false;
        }
        let positions: Option<Vec<u32>> = {
            let window = self.imp().window.borrow();
            messages
                .iter()
                .map(|message| window.position_of(*message))
                .collect()
        };
        let Some(mut positions) = positions else {
            return false;
        };
        positions.sort_unstable_by(|a, b| b.cmp(a));
        positions.dedup();
        for (n, position) in positions.into_iter().enumerate() {
            if !self.imp().window.borrow_mut().remove_at(position) {
                // Only the first can refuse: the rest are lower, and taking
                // a higher row out moved none of them.
                debug_assert_eq!(n, 0, "a later removal refused");
                return n > 0;
            }
            {
                let mut handed = self.imp().handed.borrow_mut();
                handed.remove(&position);
                let moved: Vec<(u32, MessageRow)> = handed
                    .iter()
                    .filter(|(held, _)| **held > position)
                    .map(|(held, row)| (*held, row.clone()))
                    .collect();
                for (held, _) in &moved {
                    handed.remove(held);
                }
                for (held, row) in moved {
                    handed.insert(held - 1, row);
                }
            }
            EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.items_changed(position, 1, 0);
        }
        true
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
    /// A page the feed is about to read: remembered as pending, and whether
    /// it already was (#1607). See `ListWindow::note_pending`.
    pub fn note_pending(&self, page: u32) -> bool {
        self.imp().window.borrow_mut().note_pending(page)
    }

    /// Whether `page` is on its way, from a scroll's ask or the feed's.
    pub fn is_pending(&self, page: u32) -> bool {
        self.imp().window.borrow().is_pending(page)
    }

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
        // Whatever this position answered with last time, it answers with
        // again: see `imp::MessageList::handed`. Its contents are kept current
        // by `deliver_for`, so a hit here is not a stale row, it is the same
        // row told what it now says.
        if let Some(row) = self.imp().handed.borrow().get(&position) {
            return Some(row.clone());
        }

        // Part-way through a refresh's steps, or showing the rows of a
        // question already being replaced: answer with what is here and ask
        // for nothing. The refresh fills what it makes real once it is done,
        // and the change-over replaces all of it.
        if self.imp().splicing.get() || self.imp().next.borrow().is_some() {
            let window = self.imp().window.borrow();
            if position >= window.total() {
                return None;
            }
            let row = window
                .peek(position)
                .and_then(|id| window.row_of(id))
                .and_then(MessageRow::row);
            drop(window);
            let handed = MessageRow::placeholder();
            if let Some(row) = row {
                handed.set_row(row);
            }
            self.imp()
                .handed
                .borrow_mut()
                .insert(position, handed.clone());
            return Some(handed);
        }
        let mut window = self.imp().window.borrow_mut();
        let (row, wanted) = match window.row_at(position)? {
            Lookup::Resident(row) => (row.row(), Vec::new()),
            Lookup::Missing { request } => (None, request),
        };
        drop(window);
        for page in wanted {
            // Which *position* provoked the request, which the reply-side log
            // in `postio_app::feed` cannot say. A viewport asks for positions
            // near each other; anything asking across a whole folder is the
            // loop #1534 is about, and this is the line that names who.
            tracing::debug!(position, page, "list page wanted");
            self.request(page);
        }

        // Filled before it is handed out, so the one `set_row` that could
        // reach a listener is the one `deliver_for` makes later.
        let handed = MessageRow::placeholder();
        if let Some(row) = row {
            handed.set_row(row);
        }
        self.imp()
            .handed
            .borrow_mut()
            .insert(position, handed.clone());
        Some(handed)
    }

    /// Ask the source for `page`.
    ///
    /// Called only for a page [`ListWindow`] has already confirmed needs a
    /// fresh request — deduplication against what is cached or already
    /// pending happens there, not here.
    fn request(&self, page: u32) {
        // The one place a page is asked for, which is what makes it the
        // honest place to count from (#1534). Here rather than in the source:
        // a frontend that grew a second path to the store would have to avoid
        // this function to avoid the counter.
        postio_ui::reader::cost::note_page_requested();
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
