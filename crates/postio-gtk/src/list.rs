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
//! # The windowing lives in `postio-widgets` (ADR 0043; specs/007-postio-focus T021)
//!
//! Both desktop apps draw a windowed list, over different rows, so the
//! model's behaviour -- delivery, refresh, change-over, the re-entrancy hold
//! -- is `postio_widgets::list_model`, generic over the row. What is here is
//! the classic app's row ([`Row`], [`MessageRow`]) and its `GObject`,
//! [`MessageList`], which holds that state and answers `GListModel`. Every
//! method below delegates, so nothing that names `MessageList` changed.
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

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::list::ListRow;
use postio_widgets::list_model::{ModelRow, Windowed, WindowedModel};

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

/// Where the rows come from: `postio_widgets::list_model::PageSource`, here
/// under the path the classic app's feeds already name.
pub use postio_widgets::list_model::PageSource;

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

/// The classic row is a windowed list's item: its data is a [`Row`].
impl ModelRow for MessageRow {
    type Data = Row;

    fn placeholder() -> Self {
        MessageRow::placeholder()
    }

    fn with_contents(data: Row) -> Self {
        MessageRow::new(data)
    }

    fn contents(&self) -> Option<Row> {
        self.row()
    }

    fn fill(&self, data: Row) {
        self.set_row(data);
    }

    fn id_of(data: &Row) -> MessageId {
        data.id
    }

    fn is_loaded(&self) -> bool {
        MessageRow::is_loaded(self)
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageList {
        /// Everything the model knows: the window of resident pages, the
        /// rows handed to the view, a refresh or change-over in flight. See
        /// `postio_widgets::list_model::Windowed`.
        pub core: Windowed<super::MessageRow>,
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
            // `filled` and `splicing`: see `postio_widgets::list_model::signals`.
            SIGNALS.get_or_init(postio_widgets::list_model::signals)
        }
    }

    impl ListModelImpl for MessageList {
        fn item_type(&self) -> glib::Type {
            super::MessageRow::static_type()
        }

        fn n_items(&self) -> u32 {
            self.core.n_items()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.obj().list_item(position)
        }
    }
}

/// How many `items_changed` this process has emitted from a windowed list:
/// `postio_widgets::list_model::emissions`, under its old path.
pub use postio_widgets::list_model::emissions;

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

/// The windowed list's behaviour: `postio_widgets::list_model` (T021).
impl WindowedModel for MessageList {
    type Row = MessageRow;

    fn windowed(&self) -> &Windowed<MessageRow> {
        &self.imp().core
    }
}

/// The API every caller already names. Each method is the windowed list's
/// own -- see `postio_widgets::list_model::WindowedModel` for what it does
/// and why -- under an inherent name, so no caller has to import the trait.
impl MessageList {
    /// An empty list with no source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every row the list holds, in no particular order, asking the source
    /// for nothing.
    pub fn held_rows(&self) -> Vec<Row> {
        WindowedModel::held_rows(self)
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

    /// The generation currently in force.
    pub fn generation(&self) -> u64 {
        WindowedModel::generation(self)
    }

    /// Whether the list has not yet heard from the source it was pointed at.
    pub fn is_loading(&self) -> bool {
        WindowedModel::is_loading(self)
    }

    /// Point the list at a new query: a different folder, or a search.
    pub fn set_source(&self, source: Rc<dyn PageSource>) {
        WindowedModel::set_source(self, source)
    }

    /// Point the list at a new query, keeping the rows it has until the new
    /// one's first page lands.
    pub fn replace_source(&self, source: Rc<dyn PageSource>, stash: bool) {
        WindowedModel::replace_source(self, source, stash)
    }

    /// Put back the mailbox a result set covered, and return whether there
    /// was one.
    pub fn restore(&self, source: Rc<dyn PageSource>) -> bool {
        WindowedModel::restore(self, source)
    }

    /// Hand the model a page it asked for, assuming it answers the scope
    /// currently in view.
    pub fn deliver(&self, page: u32, rows: Vec<Row>) {
        WindowedModel::deliver(self, page, rows)
    }

    /// [`deliver`](Self::deliver), checked against `generation` first.
    pub fn deliver_for(&self, generation: u64, page: u32, rows: Vec<Row>) {
        WindowedModel::deliver_for(self, generation, page, rows)
    }

    /// A page of a *mailbox* arrived, with a fresh `total`.
    pub fn deliver_page(&self, generation: u64, total: u32, page: u32, rows: Vec<Row>) {
        WindowedModel::deliver_page(self, generation, total, page, rows)
    }

    /// Give up on a page whose fetch failed, so the view can ask again.
    pub fn abandon_page(&self, generation: u64, page: u32) {
        WindowedModel::abandon_page(self, generation, page)
    }

    /// Stop waiting for `page`: its read failed and will not be tried again.
    pub fn give_up(&self, generation: u64, page: u32) {
        WindowedModel::give_up(self, generation, page)
    }

    /// Correct the row count without touching what is cached.
    pub fn set_total(&self, total: u32) {
        WindowedModel::set_total(self, total)
    }

    /// New mail landed at the top of the list.
    pub fn inserted_at_top(&self, count: u32) {
        WindowedModel::inserted_at_top(self, count)
    }

    /// A message changed in place: read, flagged, answered. Returns whether
    /// the row was resident.
    pub fn update_row(&self, row: Row) -> bool {
        WindowedModel::update_row(self, row)
    }

    /// Drop everything cached and ask again, keeping the row count and the
    /// generation.
    pub fn invalidate(&self) {
        WindowedModel::invalidate(self)
    }

    /// Re-read what is on screen and tell the view only what moved.
    pub fn refresh(&self) {
        WindowedModel::refresh(self)
    }

    /// Whether every one of `messages` is a row the list is holding.
    pub fn all_resident(&self, messages: &[MessageId]) -> bool {
        WindowedModel::all_resident(self, messages)
    }

    /// Take `messages` out of the list where they stand (#1607).
    pub fn remove_in_place(&self, messages: &[MessageId]) -> bool {
        WindowedModel::remove_in_place(self, messages)
    }

    /// How many rows are resident. The number the memory budget is about.
    pub fn resident_rows(&self) -> usize {
        WindowedModel::resident_rows(self)
    }

    /// Which resident page holds `message`, if any.
    pub fn page_of(&self, message: MessageId) -> Option<u32> {
        WindowedModel::page_of(self, message)
    }

    /// A page the feed is about to read: remembered as pending, and whether
    /// it already was (#1607).
    pub fn note_pending(&self, page: u32) -> bool {
        WindowedModel::note_pending(self, page)
    }

    /// Whether `page` is on its way, from a scroll's ask or the feed's.
    pub fn is_pending(&self, page: u32) -> bool {
        WindowedModel::is_pending(self, page)
    }

    /// Every resident page holding any of `messages`, deduplicated.
    pub fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        WindowedModel::pages_holding(self, messages)
    }

    /// Where `message` sits, among the pages currently resident.
    pub fn position_of(&self, message: MessageId) -> Option<u32> {
        WindowedModel::position_of(self, message)
    }

    /// Which pages are resident, lowest first. For tests and diagnostics.
    pub fn resident_pages(&self) -> Vec<u32> {
        WindowedModel::resident_pages(self)
    }

    /// The message at `position`, but only if its page is already here.
    pub fn peek(&self, position: u32) -> Option<MessageId> {
        WindowedModel::peek(self, position)
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
        self.imp().core.window().row_kind(message)
    }
}
