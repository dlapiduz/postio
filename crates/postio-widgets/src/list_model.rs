//! A `GListModel` windowed over paged storage, for any row type (ADR 0043;
//! specs/007-postio-focus T021).
//!
//! docs/PRODUCT.md §18 is a hard requirement — a mailbox is never loaded into
//! memory. A `GtkListView` over a windowed model asks for the rows it is about
//! to draw and nothing else, so a 100,000-message folder costs the same as a
//! 50-message one: a few hundred rows resident, and the rest a page request
//! away.
//!
//! This was postio-gtk's `MessageList`, and every behaviour below was proven
//! there. Both desktop apps draw a list this way, over different rows -- the
//! classic app's messages and conversations, Focus's conversations, digests
//! and reminders -- so the model is generic over what a position holds:
//!
//! * [`ModelRow`] is the item a view binds: a `GObject` standing for one
//!   position, filled in place when its page arrives.
//! * [`Windowed`] is the model's state, and [`WindowedModel`] its behaviour,
//!   as provided methods. An app's list is a small `GObject` implementing
//!   `gio::ListModel` that holds a `Windowed` and implements `WindowedModel`
//!   in two lines -- a `GObject` subclass cannot itself be generic, so the
//!   generic part is everything but the subclass.
//!
//! # How the pieces fit
//!
//! The view layer speaks no SQL, so the model does not fetch anything itself.
//! It says *which page it needs* through a [`PageSource`] and waits; whoever
//! implements that source calls [`WindowedModel::deliver`] on the main thread
//! when the rows arrive. Until then the positions in that page answer with a
//! placeholder, and the row widget draws a skeleton.
//!
//! The paging and generation bookkeeping lives in `postio-ui` (ADR 0019 Q5a):
//! [`ListWindow`] owns the resident pages, the LRU eviction and the
//! generation counter. What lives here is exactly what GTK's own contract
//! demands: the rows' `GObject` identity, `items_changed` emission, and the
//! `reading`/`hold` re-entrancy guard -- `GListModel::item()` must not be
//! mutated mid-call.
//!
//! # What it costs
//!
//! * [`CACHE_PAGES`] pages resident, evicted least-recently-used.
//! * One request per page, ever, until it is evicted.
//! * A page either side of the one being read, prefetched.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::gio::prelude::ListModelExt;
use gtk::glib;
use gtk::glib::prelude::*;
use postio_model::ids::MessageId;
use postio_ui::list::{ListRow, ListWindow, Lookup, Splice};

/// Rows per page.
pub use postio_ui::list::PAGE_SIZE;

/// Pages kept in memory. Everything past this is evicted least-recently-used.
pub use postio_ui::list::CACHE_PAGES;

/// Where the rows come from.
///
/// The model never blocks on this. [`request`](PageSource::request) starts the
/// work and returns; the answer arrives later through
/// [`WindowedModel::deliver`].
pub trait PageSource {
    /// How many rows the current query matches.
    fn total(&self) -> u32;

    /// Start loading `page`, whose rows are positions
    /// `page * PAGE_SIZE .. (page + 1) * PAGE_SIZE`.
    ///
    /// Called at most once per page while that page is outstanding or cached.
    fn request(&self, page: u32);
}

/// The item a windowed list hands its view: a `GObject` standing for one
/// position, a placeholder until its page arrives and filled in place after.
///
/// Filled in place, not replaced, and that is the point: the same object stays
/// where it is, so nothing above has to rediscover which row the selection is
/// on, and a page landing tells only the rows it fills rather than rebuilding
/// every widget in range (#1216).
pub trait ModelRow: ListRow + IsA<glib::Object> + Clone + 'static {
    /// What a page delivers for one position.
    type Data: Clone + PartialEq + 'static;

    /// A position whose page has not arrived.
    fn placeholder() -> Self;

    /// A loaded row.
    fn with_contents(data: Self::Data) -> Self;

    /// The row's data, if its page has arrived.
    fn contents(&self) -> Option<Self::Data>;

    /// Replace the data in place, keeping the object's identity, and tell a
    /// bound view to redraw it -- quietly, when nothing changed.
    fn fill(&self, data: Self::Data);

    /// The message a delivered row stands for: what a refresh compares by.
    fn id_of(data: &Self::Data) -> MessageId;

    /// Whether the page carrying this position has arrived.
    fn is_loaded(&self) -> bool {
        self.contents().is_some()
    }
}

/// The two signals every windowed list's `GObject` declares, for its
/// `ObjectImpl::signals`.
///
/// * `filled` -- rows that already existed now have contents. Not
///   `items_changed`: nothing moved, nothing was replaced, and saying it that
///   way costs a rebuilt widget per row in range. What it is for is everyone
///   who is not a `GtkListView` -- the reading pane following the cursor onto
///   a row that has just become real, and a seek waiting for the page
///   carrying the message it is looking for.
/// * `splicing` -- a refresh is about to tell the view what moved (`true`),
///   or has finished (`false`). Between the two a row can be taken out and put
///   back somewhere else, and a cursor that followed the position would be on
///   another message -- so whoever keeps the cursor holds it still across the
///   pair.
pub fn signals() -> Vec<glib::subclass::Signal> {
    vec![
        glib::subclass::Signal::builder("filled").build(),
        glib::subclass::Signal::builder("splicing")
            .param_types([bool::static_type()])
            .build(),
    ]
}

/// The source a list is about to show. See [`WindowedModel::replace_source`].
pub struct Next {
    source: Rc<dyn PageSource>,
    generation: u64,
    /// Keep the rows being replaced, for [`WindowedModel::restore`].
    stash: bool,
    /// A refresh was asked for while this was on its way.
    refresh: bool,
}

/// A refresh in flight: the pages it re-read, and what has come back.
///
/// See [`WindowedModel::refresh`].
pub struct Refresh<D> {
    generation: u64,
    /// Runs of pages, first and last inclusive, each re-read whole.
    runs: Vec<(u32, u32)>,
    awaiting: HashSet<u32>,
    arrived: HashMap<u32, Vec<D>>,
    total: Option<u32>,
    /// Another refresh was asked for while this one was out; run it once
    /// this one lands rather than piling a second read on top.
    again: bool,
}

/// A windowed list's state: what its `GObject` holds.
pub struct Windowed<R: ModelRow> {
    source: RefCell<Option<Rc<dyn PageSource>>>,
    window: RefCell<ListWindow<R>>,
    /// Whether the model is part-way through answering `item()`.
    ///
    /// See [`WindowedModel::hold`]: anything that would emit `items_changed`
    /// while this is set waits for the next turn of the main loop instead.
    reading: Cell<bool>,
    /// The row each position answers with, for as long as that position
    /// means the same thing.
    ///
    /// A `GListModel` says "this position holds a different row now" with
    /// `items_changed`, and `GtkListView` answers it by building a widget for
    /// every item in range — measured, on a list whose viewport holds ten
    /// rows: a page delivery of fifty built fifty (#1216). A page landing on
    /// positions that already exist is not that change. It is the same
    /// positions, holding the same messages, saying something they could not
    /// say yet, and the way to say *that* is to fill in the object the view is
    /// already holding and let it announce itself.
    ///
    /// Which only works if there is one object per position to fill. Handing
    /// out a fresh placeholder per `item()` call left the view holding objects
    /// the model had no way to reach.
    ///
    /// Dropped when a position stops meaning what it did — a new scope, an
    /// insertion at the top, an eviction — because an object kept across that
    /// would answer for the wrong message.
    handed: RefCell<HashMap<u32, R>>,
    /// A refresh whose pages are still on their way. See
    /// [`WindowedModel::refresh`].
    refresh: RefCell<Option<Refresh<R::Data>>>,
    /// Whether a refresh is part-way through telling the view what moved. A
    /// position asked for in between answers without asking the source: the
    /// step after this one may well fill it.
    splicing: Cell<bool>,
    /// The source the list is about to show, and the generation its first
    /// page will be answered under. See [`WindowedModel::replace_source`].
    next: RefCell<Option<Next>>,
    /// The mailbox a result set took the list from, kept whole so `Esc` can
    /// put it back without reading it again.
    stashed: RefCell<Option<ListWindow<R>>>,
    /// Whether the list has not yet heard from the source it was pointed at.
    /// Not the same as empty: see [`WindowedModel::is_loading`].
    loading: Cell<bool>,
}

impl<R: ModelRow> Default for Windowed<R> {
    fn default() -> Self {
        Windowed {
            source: RefCell::default(),
            window: RefCell::new(ListWindow::new()),
            reading: Cell::default(),
            handed: RefCell::default(),
            refresh: RefCell::default(),
            splicing: Cell::default(),
            next: RefCell::default(),
            stashed: RefCell::default(),
            loading: Cell::default(),
        }
    }
}

impl<R: ModelRow> Windowed<R> {
    /// The window of resident pages, borrowed for as long as the answer is
    /// read. For `postio_core::aim::RowFacts`, which asks it about rows.
    pub fn window(&self) -> Ref<'_, ListWindow<R>> {
        self.window.borrow()
    }

    /// How many positions the list has: `GListModel::n_items`.
    pub fn n_items(&self) -> u32 {
        self.window.borrow().total()
    }
}

/// A windowed list's behaviour, for the `GObject` holding its [`Windowed`]
/// state: implement [`windowed`](WindowedModel::windowed) and every method
/// here comes with it.
pub trait WindowedModel: IsA<gtk::gio::ListModel> + IsA<glib::Object> + Clone + 'static {
    /// What each position holds.
    type Row: ModelRow;

    /// The list's state.
    fn windowed(&self) -> &Windowed<Self::Row>;

    /// `GListModel::item`, for the `GObject`'s `ListModelImpl`.
    ///
    /// `row_at` may ask the source for a page, and a source that answers
    /// before it returns would change the model from inside this call.
    /// Marking the read is what lets those changes be held until it is over.
    fn list_item(&self, position: u32) -> Option<glib::Object> {
        let state = self.windowed();
        let outer = state.reading.replace(true);
        let row = self.row_at(position);
        state.reading.set(outer);
        row.map(|row| row.upcast())
    }

    /// Every row the list holds, in no particular order, asking the source
    /// for nothing. See [`postio_ui::list::ListWindow::resident`].
    fn held_rows(&self) -> Vec<<Self::Row as ModelRow>::Data> {
        self.windowed()
            .window
            .borrow()
            .resident()
            .filter_map(|entry| entry.contents())
            .collect()
    }

    /// Re-run `action` on the next turn of the main loop, because the model
    /// is part-way through answering `item()`.
    ///
    /// `PageSource::request` is called from inside the model answering
    /// `item()` — that is the whole design, and it is what keeps the fetch off
    /// the read path. What it also means is that a source which delivers
    /// before returning changes the model while a view is part-way through
    /// reading it. `GtkListView` does not survive that: it segfaults, with no
    /// message, a long way from the mistake that caused it. So the change is
    /// held for one turn of the main loop, by which time the read is over and
    /// it is merely late.
    ///
    /// Above GDK's redraw band, deliberately, and that is not a detail.
    /// `idle_add_local_once` runs at `G_PRIORITY_DEFAULT_IDLE` — 200 — and GDK
    /// paints at `GDK_PRIORITY_REDRAW`, 120. A window with something to paint
    /// every time the loop looks therefore outranks the held change for as
    /// long as the painting lasts: #1015 caught a 300-message list sitting at
    /// nought resident rows for a full five seconds, mapped and painting the
    /// whole time. `HIGH_IDLE` is the right side of the line for what this is
    /// *for*: the change has to land before the frame that would otherwise
    /// draw the state it corrects.
    fn hold(&self, action: impl FnOnce(&Self) + 'static) {
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
    /// Stamp this on a request when it is made, and pass it back to
    /// [`deliver_for`](Self::deliver_for) or [`deliver_page`](Self::deliver_page)
    /// when the answer arrives. A reply carrying an older generation is dropped
    /// rather than applied: the scope changed while it was in flight, and
    /// answering the question nobody is asking any more would fill the new
    /// scope with the old one's mail.
    fn generation(&self) -> u64 {
        let state = self.windowed();
        if let Some(next) = state.next.borrow().as_ref() {
            return next.generation;
        }
        state.window.borrow().generation()
    }

    /// Whether the list has not yet heard from the source it was pointed at.
    ///
    /// Not the same as empty, and the difference is what a person sees: an
    /// empty folder says so, and a folder whose first page is still on its
    /// way must not say it for the frame or two before the page lands.
    fn is_loading(&self) -> bool {
        self.windowed().loading.get()
    }

    /// Point the list at a new query: a different folder, or a search.
    ///
    /// Everything cached is dropped — it answered a different question — and
    /// the generation moves on, so any reply already in flight for the
    /// previous one is now stale.
    fn set_source(&self, source: Rc<dyn PageSource>) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.set_source(source));
            return;
        }
        let removed = state.window.borrow().total();
        let total = source.total();
        let generation = self.generation() + 1;

        state.next.replace(None);
        state.refresh.replace(None);
        state.loading.set(true);
        *state.source.borrow_mut() = Some(source);
        state.window.borrow_mut().adopt(total, generation);
        // Every position now stands for a different mailbox's mail.
        state.handed.borrow_mut().clear();

        items_changed(self, 0, removed, total);
    }

    /// Point the list at a new query, but keep showing the rows it has until
    /// the new one's first page lands, then change over in one step.
    ///
    /// [`set_source`](Self::set_source) empties the list at once, and the
    /// frame or two before the first page arrives drew every visible row as a
    /// skeleton -- and, in a folder switch, told the list-state pane there was
    /// no mail, so it said "Inbox is empty" on its way to showing the inbox.
    /// Keeping the old rows for those milliseconds is what a person reads as
    /// instant. The change-over is one `items_changed` with the first page
    /// already resident, so every row the view asks for then is real.
    ///
    /// [`generation`](Self::generation) moves on now, so the first page is
    /// requested and answered under the new question; a page of the old one
    /// still in flight is dropped. The old rows ask for nothing while they
    /// wait. A list showing nothing has nothing to keep, and changes over at
    /// once.
    ///
    /// `stash` keeps the rows being replaced, for [`restore`](Self::restore)
    /// -- a mailbox a search is about to cover. Only the first stash counts:
    /// refining the query replaces one result set with another, and the
    /// mailbox underneath is still the one to go back to.
    fn replace_source(&self, source: Rc<dyn PageSource>, stash: bool) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.replace_source(source, stash));
            return;
        }
        state.refresh.replace(None);
        if !stash {
            state.stashed.replace(None);
        }
        let showing = {
            let window = state.window.borrow();
            window.total() > 0 && !window.resident_pages().is_empty()
        };
        if !showing {
            self.set_source(source);
            return;
        }
        let generation = self.generation() + 1;
        state.next.replace(Some(Next {
            source,
            generation,
            stash,
            refresh: false,
        }));
        state.loading.set(true);
    }

    /// Put back the mailbox a result set covered, rows and all, and return
    /// whether there was one.
    ///
    /// No read is needed to show it: its pages are the ones it had, under a
    /// new generation so whatever the result set still has in flight is
    /// dropped. The caller refreshes it afterwards, which re-reads only what
    /// is on screen and moves only what changed while the search was up. A
    /// result set still waiting for its first page never took the list, so
    /// the mailbox is simply still there.
    fn restore(&self, source: Rc<dyn PageSource>) -> bool {
        let state = self.windowed();
        if state.reading.get() {
            return false;
        }
        if state.next.borrow().is_some() && state.stashed.borrow().is_none() {
            // Nothing was swapped out; drop the swap that was coming.
            let generation = self.generation() + 1;
            state.next.replace(None);
            state.loading.set(false);
            *state.source.borrow_mut() = Some(source);
            // A new generation for the rows already here, so the result
            // set's first page, still out, cannot land on them.
            state.window.borrow_mut().renumber(generation);
            return true;
        }
        let Some(mut kept) = state.stashed.take() else {
            return false;
        };
        let generation = self.generation() + 1;
        let total = kept.total();
        kept.renumber(generation);
        let removed = state.window.borrow().total();
        state.next.replace(None);
        state.refresh.replace(None);
        state.loading.set(false);
        *state.source.borrow_mut() = Some(source);
        *state.window.borrow_mut() = kept;
        state.handed.borrow_mut().clear();
        swap_in(self, removed, total);
        true
    }

    /// Hand the model a page it asked for, assuming it answers the scope
    /// currently in view.
    ///
    /// For a caller with no generation to compare — a test double, or
    /// [`PageSource::request`] answering synchronously — this always applies.
    /// A caller that captured the generation at request time and needs a
    /// stale answer dropped instead wants [`deliver_for`](Self::deliver_for).
    ///
    /// Rows already resident for the same message keep their `GObject`
    /// ([`ListRow::reconcile`]), so a redelivered page does not invalidate
    /// anything holding onto them.
    fn deliver(&self, page: u32, rows: Vec<<Self::Row as ModelRow>::Data>) {
        let generation = self.generation();
        self.deliver_for(generation, page, rows);
    }

    /// The same as [`deliver`](Self::deliver), but `generation` is checked
    /// against the one currently in force first.
    fn deliver_for(&self, generation: u64, page: u32, rows: Vec<<Self::Row as ModelRow>::Data>) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.deliver_for(generation, page, rows));
            return;
        }
        let waiting = state
            .next
            .borrow()
            .as_ref()
            .map(|next| (next.generation, next.source.total()));
        if let Some((next, total)) = waiting {
            if generation == next {
                change_over(self, total, page, rows);
            }
            return;
        }
        let Some(rows) = take_for_refresh(self, generation, page, rows, None) else {
            return;
        };
        // Fill the objects the view is already holding for these positions.
        // Collected before any of them is told, because a handler is free to
        // ask the model for a row and `row_at` takes this borrow mutably.
        let start = page * PAGE_SIZE;
        let filling: Vec<(Self::Row, <Self::Row as ModelRow>::Data)> = {
            let handed = state.handed.borrow();
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

        let items: Vec<Self::Row> = rows.into_iter().map(Self::Row::with_contents).collect();
        let delivered = state.window.borrow_mut().deliver(generation, page, items);
        if delivered.stale {
            return;
        }
        // The first answer ends the wait even when it brought no rows: an
        // empty folder is only known to be empty once it has said so, and
        // whoever draws that has to hear about it.
        let answered = state.loading.replace(false);
        let filled = !filling.is_empty() || answered;
        for (held, row) in filling {
            held.fill(row);
        }
        // An evicted page's positions are not on screen -- that is what makes
        // them evictable -- so the objects standing for them can go too, and
        // must, or a list scrolled through a mailbox would keep one per row it
        // passed.
        if !delivered.evicted.is_empty() {
            let mut handed = state.handed.borrow_mut();
            for page in &delivered.evicted {
                let first = page * PAGE_SIZE;
                for position in first..first + PAGE_SIZE {
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
        // every row in range.
        let _ = delivered.changed;
    }

    /// A page of a *mailbox* arrived: a fresh `total` alongside this page's
    /// `rows`, generation-checked as one decision.
    ///
    /// What [`deliver_for`](Self::deliver_for) is for a result set's page,
    /// which carries no total of its own — a mailbox's answer always does,
    /// and the total is applied first, so a page delivered before the list
    /// knows how long it is would not be dropped by the very count it just
    /// supplied.
    fn deliver_page(
        &self,
        generation: u64,
        total: u32,
        page: u32,
        rows: Vec<<Self::Row as ModelRow>::Data>,
    ) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.deliver_page(generation, total, page, rows));
            return;
        }
        let waiting = state.next.borrow().as_ref().map(|next| next.generation);
        if let Some(next) = waiting {
            if generation == next {
                change_over(self, total, page, rows);
            }
            return;
        }
        if generation != self.generation() {
            return;
        }
        let Some(rows) = take_for_refresh(self, generation, page, rows, Some(total)) else {
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
    /// nothing can clear. Behind the same `reading` guard as
    /// [`deliver_for`](Self::deliver_for): `GListModel::item()` must not be
    /// re-entered, and a failure can arrive mid-call exactly as a delivery
    /// can.
    fn abandon_page(&self, generation: u64, page: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.abandon_page(generation, page));
            return;
        }
        state.window.borrow_mut().abandon(generation, page);
    }

    /// Stop waiting for `page`: its read failed and will not be tried again.
    ///
    /// A refresh that was holding its other pages for this one applies what
    /// it has, and a change-over waiting on a first page that is never coming
    /// changes over to the count the source knows, rows to follow when they
    /// are asked for -- rather than showing the folder it left for the rest
    /// of the session.
    fn give_up(&self, generation: u64, page: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.give_up(generation, page));
            return;
        }
        let waiting = state
            .next
            .borrow()
            .as_ref()
            .map(|next| (next.generation, next.source.total()));
        if let Some((next, total)) = waiting {
            if generation == next {
                change_over(self, total, page, Vec::new());
                state.window.borrow_mut().abandon(generation, page);
            }
            return;
        }
        state.window.borrow_mut().abandon(generation, page);
        // Given up on under the question in force: the answer is that there
        // is nothing, which ends the wait exactly as a first page that brought
        // no rows does in `deliver_for`. A search with no hits gets here -- it
        // has no page 0 to ask for -- and without this a list that was empty
        // when it arrived stayed "loading", so the pane withheld "nothing
        // matched" for good.
        if generation == self.generation() && state.loading.replace(false) {
            self.emit_by_name::<()>("filled", &[]);
        }
        let complete = {
            let mut slot = state.refresh.borrow_mut();
            match slot.as_mut() {
                Some(refresh) if refresh.generation == generation => {
                    refresh.awaiting.remove(&page) && refresh.awaiting.is_empty()
                }
                _ => false,
            }
        };
        if complete {
            apply_refresh(self);
        }
    }

    /// Correct the row count without touching what is cached.
    ///
    /// For a total that shrank or grew at the *end* of the list — a mailbox
    /// recount. New mail arriving at the top is
    /// [`inserted_at_top`](Self::inserted_at_top).
    fn set_total(&self, total: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.set_total(total));
            return;
        }
        // Not `if let Some(..) = ...borrow_mut()...` — a temporary borrowed
        // directly in an `if let` condition lives for the whole statement, so
        // it would still be held when `items_changed` below re-enters `item()`
        // synchronously (a listening `GtkListView` does). Binding it first
        // ends that statement, and the borrow, before the signal fires.
        let change = state.window.borrow_mut().set_total(total);
        // A recount moves the *end* of the list. Positions before it still
        // mean what they did, so their objects stay; the ones past the new end
        // do not exist any more.
        state
            .handed
            .borrow_mut()
            .retain(|position, _| *position < total);
        if let Some((position, removed, added)) = change {
            items_changed(self, position, removed, added);
        }
    }

    /// New mail landed at the top of the list.
    ///
    /// Every row shifts down by `count`, which misaligns every cached page
    /// against its positions, so the cache is dropped and refetched. What it
    /// deliberately does *not* do is reset the model: `items_changed` at
    /// position 0 is an insertion, so a selection model moves the selection
    /// down with the row it is on and the view keeps its scroll anchor.
    fn inserted_at_top(&self, count: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.inserted_at_top(count));
            return;
        }
        state.refresh.replace(None);
        let inserted = state.window.borrow_mut().inserted_at_top(count);
        if inserted {
            // Every row shifted down by `count`, so every position stands for
            // a different message than the object held for it does.
            state.handed.borrow_mut().clear();
            items_changed(self, 0, 0, count);
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
    fn update_row(&self, row: <Self::Row as ModelRow>::Data) -> bool {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| {
                list.update_row(row);
            });
            return false;
        }
        let incoming = Self::Row::with_contents(row.clone());
        let Some(position) = state.window.borrow_mut().update(incoming) else {
            return false;
        };
        // Same position, same message, new contents — so the row says so for
        // itself and the model stays quiet. Only a row nothing is holding for
        // needs telling through `items_changed`.
        let held = state.handed.borrow().get(&position).cloned();
        match held {
            Some(held) => held.fill(row),
            None => items_changed(self, position, 1, 1),
        }
        true
    }

    /// Drop everything cached and ask again, keeping the row count and the
    /// generation.
    ///
    /// The blunt instrument, for when the order itself changed but the
    /// question being answered has not — a request already in flight from
    /// before this call is still answering it, so it is not stale.
    fn invalidate(&self) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(|list| list.invalidate());
            return;
        }
        state.refresh.replace(None);
        let total = state.window.borrow_mut().invalidate();
        // The order is what changed, so a position no longer means what it did.
        state.handed.borrow_mut().clear();
        items_changed(self, 0, total, total);
    }

    /// Re-read what is on screen and tell the view only what moved.
    ///
    /// What a resync, a backfill batch, an archive in an aggregate view or new
    /// mail in a unified one all come to: the membership or the order may have
    /// moved, and the list does not know how. The rows stay. The pages
    /// somebody is looking at -- any page holding a row the view or the
    /// selection still holds -- are read again, and until every one of them
    /// has landed the list goes on answering with what it had. Then each run
    /// of them is compared with what came back, by message id
    /// ([`postio_ui::list::splices`]): a row still there in the same order
    /// stays where it is and is told what it says now, and everything else is
    /// a removal or an insertion *at its position*, so the view keeps its
    /// scroll anchor and every other row's widget. Nothing moved is nothing
    /// emitted. Pages nobody is looking at are dropped, to be read at their
    /// new offsets when they are wanted.
    ///
    /// One refresh at a time: asked for again while one is out, it runs once
    /// more after it lands, however many times it was asked. The count comes
    /// back with the pages and moves the end of the list.
    fn refresh(&self) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(|list| list.refresh());
            return;
        }
        // A change-over's first page may have been read before whatever this
        // refresh is for; refresh the new rows once they are here.
        if let Some(next) = state.next.borrow_mut().as_mut() {
            next.refresh = true;
            return;
        }
        if let Some(refresh) = state.refresh.borrow_mut().as_mut() {
            refresh.again = true;
            return;
        }
        let (generation, resident) = {
            let window = state.window.borrow();
            (window.generation(), window.resident_pages())
        };
        let mut watched: Vec<u32> = state
            .handed
            .borrow()
            .iter()
            // One reference is the map's own. Anything more is the view's row
            // for that position, or the selection's.
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
                let mut window = state.window.borrow_mut();
                window.retain_pages(|_| false);
                window.is_pending(0)
            };
            state
                .handed
                .borrow_mut()
                .retain(|_, row| row.ref_count() > 1);
            if !pending {
                request(self, 0);
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
        state.refresh.replace(Some(Refresh {
            generation,
            runs,
            awaiting: watched.iter().copied().collect(),
            arrived: HashMap::new(),
            total: None,
            again: false,
        }));
        for page in watched {
            state.window.borrow_mut().note_pending(page);
            request(self, page);
        }
    }

    /// Whether every one of `messages` is a row the list is holding.
    fn all_resident(&self, messages: &[MessageId]) -> bool {
        let window = self.windowed().window.borrow();
        !messages.is_empty()
            && messages
                .iter()
                .all(|message| window.position_of(*message).is_some())
    }

    /// Take `messages` out of the list where they stand, each row after them
    /// moving up, rather than reloading it (#1607).
    ///
    /// `false`, with nothing changed, unless every one is a resident row that
    /// [`ListWindow::remove_at`] can take out: the caller reloads then.
    /// Removed from the bottom up, so no removal moves a position another is
    /// still to use, and announced one row at a time --
    /// `items_changed(position, 1, 0)` -- so the list view keeps every other
    /// row's widget, and a cursor on a removed row lands on the one that moved
    /// into its place.
    fn remove_in_place(&self, messages: &[MessageId]) -> bool {
        let state = self.windowed();
        if state.reading.get() {
            return false;
        }
        let positions: Option<Vec<u32>> = {
            let window = state.window.borrow();
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
            if !state.window.borrow_mut().remove_at(position) {
                // Only the first can refuse: the rest are lower, and taking a
                // higher row out moved none of them.
                debug_assert_eq!(n, 0, "a later removal refused");
                return n > 0;
            }
            {
                let mut handed = state.handed.borrow_mut();
                handed.remove(&position);
                let moved: Vec<(u32, Self::Row)> = handed
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
            items_changed(self, position, 1, 0);
        }
        true
    }

    /// How many rows are resident. The number the memory budget is about.
    fn resident_rows(&self) -> usize {
        self.windowed().window.borrow().resident_rows()
    }

    /// Which resident page holds `message`, if any.
    ///
    /// The cheap half of reacting to a change: a message that changed
    /// somewhere off screen needs nothing done, and one that is on screen
    /// costs a refetch of its page rather than of the folder.
    fn page_of(&self, message: MessageId) -> Option<u32> {
        self.windowed().window.borrow().page_of(message)
    }

    /// A page the feed is about to read: remembered as pending, and whether
    /// it already was (#1607). See `ListWindow::note_pending`.
    fn note_pending(&self, page: u32) -> bool {
        self.windowed().window.borrow_mut().note_pending(page)
    }

    /// Whether `page` is on its way, from a scroll's ask or the feed's.
    fn is_pending(&self, page: u32) -> bool {
        self.windowed().window.borrow().is_pending(page)
    }

    /// Every resident page holding any of `messages`, deduplicated: the bulk
    /// form of [`page_of`](Self::page_of), so a burst of changes costs one
    /// request per affected page rather than one per message.
    fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        self.windowed().window.borrow().pages_holding(messages)
    }

    /// Where `message` sits, among the pages currently resident.
    ///
    /// `None` covers both "not in this mailbox" and "resident but not fetched
    /// yet" — a caller that wants to put the cursor on a message it did not
    /// just deliver itself cannot tell those apart and has to treat them the
    /// same: ask for the page, and try again once it answers.
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.windowed().window.borrow().position_of(message)
    }

    /// Which pages are resident, lowest first. For tests and diagnostics.
    fn resident_pages(&self) -> Vec<u32> {
        self.windowed().window.borrow().resident_pages()
    }

    /// The message at `position`, but only if its page is already here.
    ///
    /// [`list_item`](Self::list_item) fetches what it does not have, which is
    /// right for drawing — a row on screen has to become real — and wrong for
    /// answering "what is in this range". A Shift-click across ten thousand
    /// rows must not ask the store for ten thousand rows.
    fn peek(&self, position: u32) -> Option<MessageId> {
        self.windowed().window.borrow().peek(position)
    }

    /// The row at `position`, fetching its page if it is not resident.
    fn row_at(&self, position: u32) -> Option<Self::Row> {
        let state = self.windowed();
        // Whatever this position answered with last time, it answers with
        // again: see `Windowed::handed`. Its contents are kept current by
        // `deliver_for`, so a hit here is not a stale row, it is the same row
        // told what it now says.
        if let Some(row) = state.handed.borrow().get(&position) {
            return Some(row.clone());
        }

        // Part-way through a refresh's steps, or showing the rows of a
        // question already being replaced: answer with what is here and ask
        // for nothing. The refresh fills what it makes real once it is done,
        // and the change-over replaces all of it.
        if state.splicing.get() || state.next.borrow().is_some() {
            let window = state.window.borrow();
            if position >= window.total() {
                return None;
            }
            let row = window
                .peek(position)
                .and_then(|id| window.row_of(id))
                .and_then(|row| row.contents());
            drop(window);
            let handed = Self::Row::placeholder();
            if let Some(row) = row {
                handed.fill(row);
            }
            state.handed.borrow_mut().insert(position, handed.clone());
            return Some(handed);
        }
        let mut window = state.window.borrow_mut();
        let (row, wanted) = match window.row_at(position)? {
            Lookup::Resident(row) => (row.contents(), Vec::new()),
            Lookup::Missing { request } => (None, request),
        };
        drop(window);
        for page in wanted {
            // Which *position* provoked the request, which the reply-side log
            // cannot say. A viewport asks for positions near each other;
            // anything asking across a whole folder is the loop #1534 is
            // about, and this is the line that names who.
            tracing::debug!(position, page, "list page wanted");
            request(self, page);
        }

        // Filled before it is handed out, so the one `fill` that could
        // reach a listener is the one `deliver_for` makes later.
        let handed = Self::Row::placeholder();
        if let Some(row) = row {
            handed.fill(row);
        }
        state.handed.borrow_mut().insert(position, handed.clone());
        Some(handed)
    }
}

/// Tell the view that positions changed.
fn items_changed<M: WindowedModel>(model: &M, position: u32, removed: u32, added: u32) {
    model
        .upcast_ref::<gtk::gio::ListModel>()
        .items_changed(position, removed, added);
}

/// Change over to the source `replace_source` was waiting on, now that a page
/// of it has landed.
fn change_over<M: WindowedModel>(
    model: &M,
    total: u32,
    page: u32,
    rows: Vec<<M::Row as ModelRow>::Data>,
) {
    let state = model.windowed();
    let Some(next) = state.next.take() else {
        return;
    };
    let total = total.max(rows.len() as u32 + page * PAGE_SIZE);
    let removed = state.window.borrow().total();
    *state.source.borrow_mut() = Some(next.source);
    state.loading.set(false);
    {
        let mut fresh = ListWindow::new();
        fresh.adopt(total, next.generation);
        fresh.deliver(
            next.generation,
            page,
            rows.into_iter().map(M::Row::with_contents).collect(),
        );
        let old = std::mem::replace(&mut *state.window.borrow_mut(), fresh);
        if next.stash && state.stashed.borrow().is_none() {
            state.stashed.replace(Some(old));
        }
    }
    state.handed.borrow_mut().clear();
    swap_in(model, removed, total);
    if next.refresh {
        model.refresh();
    }
}

/// Tell the view that every one of its `removed` rows is now one of a
/// different list's `total`, which the window already holds.
///
/// In two steps rather than one, and the split is what keeps a folder switch
/// cheap. `GtkSingleSelection`, told its selected row was replaced, looks for
/// the item it had among *every* row added -- one `item()` each -- and every
/// one of those asks for its page: opening a 3,618-conversation folder asked
/// for 73. So the replace covers only the rows the window already holds from
/// the top, and the rest arrive as an insertion at the end, which the
/// selection has no reason to search.
fn swap_in<M: WindowedModel>(model: &M, removed: u32, total: u32) {
    let head = {
        let mut window = model.windowed().window.borrow_mut();
        let head = (0..total)
            .take_while(|position| window.peek(*position).is_some())
            .count() as u32;
        window.set_total(head);
        head
    };
    items_changed(model, 0, removed, head);
    model.set_total(total);
}

/// Hold a page a refresh is waiting for, applying the refresh once the last
/// of them is in. Hands `rows` back when they are not a refresh's.
fn take_for_refresh<M: WindowedModel>(
    model: &M,
    generation: u64,
    page: u32,
    rows: Vec<<M::Row as ModelRow>::Data>,
    total: Option<u32>,
) -> Option<Vec<<M::Row as ModelRow>::Data>> {
    let state = model.windowed();
    let complete = {
        let mut slot = state.refresh.borrow_mut();
        let Some(refresh) = slot.as_mut() else {
            return Some(rows);
        };
        if refresh.generation != generation || !refresh.awaiting.remove(&page) {
            return Some(rows);
        }
        if state.window.borrow_mut().answered(generation, page) {
            refresh.arrived.insert(page, rows);
        }
        if total.is_some() {
            refresh.total = total;
        }
        refresh.awaiting.is_empty()
    };
    if complete {
        apply_refresh(model);
    }
    None
}

/// Tell the view what a refresh found. See [`WindowedModel::refresh`].
fn apply_refresh<M: WindowedModel>(model: &M) {
    let state = model.windowed();
    let Some(mut refresh) = state.refresh.take() else {
        return;
    };
    if refresh.generation != model.generation() {
        return;
    }
    let asked = refresh.runs.clone();
    let in_run = move |page: u32| {
        asked
            .iter()
            .any(|(first, last)| (*first..=*last).contains(&page))
    };
    // Pages nobody was looking at are one row out for every row this moves
    // above them, so they go. What the view still holds for them stays: it is
    // the same message either way.
    {
        let in_run = in_run.clone();
        state.window.borrow_mut().retain_pages(in_run);
    }
    state
        .handed
        .borrow_mut()
        .retain(|position, row| in_run(position / PAGE_SIZE) || row.ref_count() > 1);

    state.splicing.set(true);
    model.emit_by_name::<()>("splicing", &[&true]);
    let mut broken = false;
    // Bottom up, so a run's steps never move a position a later run is still
    // to use.
    for (first, last) in refresh.runs.iter().rev() {
        if !(*first..=*last).all(|page| refresh.arrived.contains_key(&page)) {
            continue;
        }
        let start = first * PAGE_SIZE;
        let incoming: Vec<<M::Row as ModelRow>::Data> = (*first..=*last)
            .flat_map(|page| refresh.arrived.remove(&page).unwrap_or_default())
            .collect();
        let held: Vec<Option<MessageId>> = {
            let window = state.window.borrow();
            (start..(last + 1) * PAGE_SIZE)
                .map_while(|position| window.peek(position).map(Some))
                .collect()
        };
        let ids: Vec<MessageId> = incoming.iter().map(M::Row::id_of).collect();
        for step in postio_ui::list::splices(&held, &ids) {
            let (position, removed, rows) = match step {
                Splice::Remove { at, count } => (start + at, count, Vec::new()),
                Splice::Insert { at, count, from } => (
                    start + at,
                    0,
                    incoming[from..from + count as usize]
                        .iter()
                        .cloned()
                        .map(M::Row::with_contents)
                        .collect(),
                ),
            };
            let added = rows.len() as u32;
            if !state.window.borrow_mut().splice(position, removed, rows) {
                broken = true;
                break;
            }
            shift_handed(model, position, removed, added);
            items_changed(model, position, removed, added);
        }
        if broken {
            break;
        }
        // What stayed: the same messages in the same places, told what they
        // say now. Quiet for a row that says what it said.
        for (offset, row) in incoming.into_iter().enumerate() {
            let position = start + offset as u32;
            state
                .window
                .borrow_mut()
                .update(M::Row::with_contents(row.clone()));
            let held = state.handed.borrow().get(&position).cloned();
            if let Some(held) = held {
                held.fill(row);
            }
        }
    }
    state.splicing.set(false);

    if broken {
        // A step the window could not take: it and the view no longer agree
        // about a position, and only starting over is honest.
        model.emit_by_name::<()>("splicing", &[&false]);
        model.invalidate();
        request(model, 0);
        return;
    }

    // A position the view asked for between two steps was answered with a
    // placeholder; fill the ones a later step made real, and ask for the
    // rest.
    let waiting: Vec<(u32, M::Row)> = state
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
            let mut window = state.window.borrow_mut();
            match window.row_at(position) {
                Some(Lookup::Resident(row)) => Ok(row.contents()),
                Some(Lookup::Missing { request }) => Err(request),
                None => Ok(None),
            }
        };
        match lookup {
            Ok(Some(row)) => {
                placeholder.fill(row);
                filled = true;
            }
            Ok(None) => {}
            Err(request) => wanted.extend(request),
        }
    }
    for page in wanted {
        request(model, page);
    }
    if let Some(total) = refresh.total {
        model.set_total(total);
    }
    model.emit_by_name::<()>("splicing", &[&false]);
    if filled {
        model.emit_by_name::<()>("filled", &[]);
    }
    if refresh.again {
        model.refresh();
    }
}

/// Re-key the objects handed to the view for a splice at `position`: the
/// `removed` there go, and every one after moves by the difference.
fn shift_handed<M: WindowedModel>(model: &M, position: u32, removed: u32, added: u32) {
    let mut handed = model.windowed().handed.borrow_mut();
    let before = std::mem::take(&mut *handed);
    for (held, row) in before {
        if held < position {
            handed.insert(held, row);
        } else if held >= position + removed {
            handed.insert(held - removed + added, row);
        }
    }
}

/// Ask the source for `page`.
///
/// Called only for a page [`ListWindow`] has already confirmed needs a fresh
/// request — deduplication against what is cached or already pending happens
/// there, not here. The one place a page is asked for, which is what makes it
/// the honest place to count from (#1534).
fn request<M: WindowedModel>(model: &M, page: u32) {
    postio_ui::reader::cost::note_page_requested();
    let source = model.windowed().source.borrow().clone();
    if let Some(source) = source {
        source.request(page);
    }
}
