//! A `GListModel` windowed over paged storage, for any row type: a mailbox is never loaded into
//! memory.
//! The model asks a [`PageSource`] for the page it needs and waits; whoever implements it calls
//! [`WindowedModel::deliver`]
//! on the main thread. Paging and generation bookkeeping live in `postio-ui`; here is what GTK's
//! contract demands: row
//! `GObject` identity, `items_changed` emission and the `reading`/`hold` re-entrancy guard.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::gio::prelude::ListModelExt;
use gtk::glib;
use gtk::glib::prelude::*;
use postio_model::ids::MessageId;
use postio_ui::list::{ListRow, ListWindow, Lookup, Splice};

pub use postio_ui::list::PAGE_SIZE;

pub use postio_ui::list::CACHE_PAGES;

/// The model never blocks on this: `request` starts the work and the answer arrives through
/// [`WindowedModel::deliver`].
pub trait PageSource {
    fn total(&self) -> u32;

    /// Called at most once per page while that page is outstanding or cached.
    fn request(&self, page: u32);
}

/// Filled in place, not replaced, so the same object stays where it is and a page landing tells
/// only the rows it fills
/// rather than rebuilding every widget in range (#1216).
pub trait ModelRow: ListRow + IsA<glib::Object> + Clone + 'static {
    type Data: Clone + PartialEq + 'static;

    fn placeholder() -> Self;

    fn with_contents(data: Self::Data) -> Self;

    fn contents(&self) -> Option<Self::Data>;

    fn fill(&self, data: Self::Data);

    /// What a refresh compares by, so a thread's row stays the same row when a reply changes its
    /// message.
    fn key_of(data: &Self::Data) -> postio_ui::list::RowKey;

    fn is_loaded(&self) -> bool {
        self.contents().is_some()
    }
}

/// `filled`: rows that already existed now have contents; not `items_changed`, which costs a
/// rebuilt widget per row in range.
/// `splicing`: a refresh is about to tell the view what moved (`true`) or has finished (`false`);
/// whoever keeps the cursor
/// holds it still across the pair.
pub fn signals() -> Vec<glib::subclass::Signal> {
    vec![
        glib::subclass::Signal::builder("filled").build(),
        glib::subclass::Signal::builder("splicing")
            .param_types([bool::static_type()])
            .build(),
    ]
}

pub struct Next {
    source: Rc<dyn PageSource>,
    generation: u64,
    stash: bool,
    refresh: bool,
}

pub struct Refresh<D> {
    generation: u64,
    runs: Vec<(u32, u32)>,
    awaiting: HashSet<u32>,
    arrived: HashMap<u32, Vec<D>>,
    total: Option<u32>,
    /// Another refresh was asked for while this one was out: run it once this lands.
    again: bool,
}

pub struct Windowed<R: ModelRow> {
    source: RefCell<Option<Rc<dyn PageSource>>>,
    window: RefCell<ListWindow<R>>,
    /// While set, anything that would emit `items_changed` waits for the next main-loop turn (see
    /// `hold`).
    reading: Cell<bool>,
    /// The row each position answers with while that position means the same thing. A page landing
    /// on existing positions is
    /// filled in on the object the view holds: `items_changed` makes `GtkListView` build a widget
    /// per item in range (a page
    /// of fifty built fifty, #1216). Dropped when a position stops meaning what it did (new scope,
    /// insertion at the top, eviction).
    handed: RefCell<HashMap<u32, R>>,
    refresh: RefCell<Option<Refresh<R::Data>>>,
    /// A position asked for in between answers without asking the source: the next step may fill
    /// it.
    splicing: Cell<bool>,
    next: RefCell<Option<Next>>,
    stashed: RefCell<Option<ListWindow<R>>>,
    /// Not the same as empty: see [`WindowedModel::is_loading`].
    loading: Cell<bool>,
    /// A read of the top already on its way may predate what the refresh was for, so the top is
    /// read again once it lands.
    top_again: Cell<bool>,
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
            top_again: Cell::default(),
        }
    }
}

impl<R: ModelRow> Windowed<R> {
    pub fn window(&self) -> Ref<'_, ListWindow<R>> {
        self.window.borrow()
    }

    pub fn n_items(&self) -> u32 {
        self.window.borrow().total()
    }
}

pub trait WindowedModel: IsA<gtk::gio::ListModel> + IsA<glib::Object> + Clone + 'static {
    type Row: ModelRow;

    fn windowed(&self) -> &Windowed<Self::Row>;

    /// `row_at` may ask the source for a page, and a source that answers before returning would
    /// change the model inside this
    /// call: marking the read holds those changes until it is over.
    fn list_item(&self, position: u32) -> Option<glib::Object> {
        let state = self.windowed();
        let outer = state.reading.replace(true);
        let row = self.row_at(position);
        state.reading.set(outer);
        row.map(|row| row.upcast())
    }

    fn held_rows(&self) -> Vec<<Self::Row as ModelRow>::Data> {
        self.windowed()
            .window
            .borrow()
            .resident()
            .filter_map(|entry| entry.contents())
            .collect()
    }

    /// `PageSource::request` runs inside `item()`, so a source that delivers before returning
    /// changes the model mid-read, which
    /// segfaults `GtkListView` far from the mistake: the change is held one main-loop turn.
    /// Above GDK's redraw band on purpose: `idle_add_local_once` runs at priority 200 and GDK
    /// paints at
    /// `GDK_PRIORITY_REDRAW` (120), so a window painting every turn starves it (#1015: a list at
    /// zero resident rows for
    /// five seconds). `HIGH_IDLE` lands before the frame that would draw the state it corrects.
    fn hold(&self, action: impl FnOnce(&Self) + 'static) {
        let list = self.clone();
        // `_full` because only it takes a priority; the `Option` lets an `FnOnce` fit its `FnMut`.
        let mut action = Some(action);
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            if let Some(action) = action.take() {
                action(&list);
            }
            glib::ControlFlow::Break
        });
    }

    /// Stamp it on a request and pass it back to `deliver_for`/`deliver_page`: a reply with an
    /// older generation is dropped,
    /// since the scope changed in flight.
    fn generation(&self) -> u64 {
        let state = self.windowed();
        if let Some(next) = state.next.borrow().as_ref() {
            return next.generation;
        }
        state.window.borrow().generation()
    }

    /// Not the same as empty: a folder whose first page is still on its way must not say it is
    /// empty for a frame or two.
    fn is_loading(&self) -> bool {
        self.windowed().loading.get()
    }

    /// Everything cached is dropped and the generation moves on, so a reply in flight for the
    /// previous query is stale.
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
        state.top_again.set(false);
        state.loading.set(true);
        *state.source.borrow_mut() = Some(source);
        state.window.borrow_mut().adopt(total, generation);
        state.handed.borrow_mut().clear();

        items_changed(self, 0, removed, total);
    }

    /// Keeps showing the current rows until the new query's first page lands, then changes over in
    /// one `items_changed`:
    /// `set_source` empties at once, so a folder switch drew skeletons and told the list-state pane
    /// "Inbox is empty".
    /// The generation moves on now; the old rows ask for nothing while they wait. `stash` keeps
    /// them for
    /// [`restore`](Self::restore), and only the first stash counts: refining a query leaves the
    /// mailbox underneath the one to go back to.
    fn replace_source(&self, source: Rc<dyn PageSource>, stash: bool) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.replace_source(source, stash));
            return;
        }
        state.refresh.replace(None);
        state.top_again.set(false);
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

    /// No read is needed: its pages are kept under a new generation, so whatever the result set has
    /// in flight is dropped.
    /// The caller refreshes it afterwards.
    fn restore(&self, source: Rc<dyn PageSource>) -> bool {
        let state = self.windowed();
        if state.reading.get() {
            return false;
        }
        if state.next.borrow().is_some() && state.stashed.borrow().is_none() {
            let generation = self.generation() + 1;
            state.next.replace(None);
            state.loading.set(false);
            *state.source.borrow_mut() = Some(source);
            // A new generation, so the result set's first page, still out, cannot land on these
            // rows.
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

    /// Assumes the page answers the scope in view, for callers with no generation to compare.
    /// Resident rows for the same
    /// message keep their `GObject` ([`ListRow::reconcile`]).
    fn deliver(&self, page: u32, rows: Vec<<Self::Row as ModelRow>::Data>) {
        let generation = self.generation();
        self.deliver_for(generation, page, rows);
    }

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
        // Collected before any is told: a handler may ask the model for a row, and `row_at` takes
        // this borrow mutably.
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
        if page == 0 && state.top_again.replace(false) {
            request(self, 0);
        }
        // The first answer ends the wait even with no rows: an empty folder is known empty only
        // once it has said so.
        let answered = state.loading.replace(false);
        let filled = !filling.is_empty() || answered;
        for (held, row) in filling {
            held.fill(row);
        }
        // An evicted page's positions are off screen, so their objects go too, or a scrolled
        // mailbox keeps one per row passed.
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
        // Nothing structural happened; announcing it through the model would make `GtkListView`
        // rebuild a widget per row in range.
        let _ = delivered.changed;
    }

    /// A mailbox page carries a fresh `total`, applied first so the page is not dropped by the very
    /// count it supplies.
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

    /// Without this a failed page stays outstanding for the window's life and its positions draw
    /// skeletons nothing can clear.
    /// Behind the same `reading` guard as `deliver_for`: a failure can arrive mid-call.
    fn abandon_page(&self, generation: u64, page: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.abandon_page(generation, page));
            return;
        }
        state.window.borrow_mut().abandon(generation, page);
    }

    /// A refresh holding its other pages for this one applies what it has, and a change-over
    /// waiting on a first page that
    /// never comes changes over to the count the source knows.
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
        // Nothing is the answer, which ends the wait as an empty first page does: a search with no
        // hits has no page 0, and the
        // pane withheld "nothing matched" for good.
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

    /// For a total that shrank or grew at the end of the list; new mail at the top is
    /// [`inserted_at_top`](Self::inserted_at_top).
    fn set_total(&self, total: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.set_total(total));
            return;
        }
        // Bound first: a temporary borrowed in an `if let` condition lives for the whole statement,
        // and `items_changed` below
        // re-enters `item()` synchronously.
        let change = state.window.borrow_mut().set_total(total);
        state
            .handed
            .borrow_mut()
            .retain(|position, _| *position < total);
        if let Some((position, removed, added)) = change {
            items_changed(self, position, removed, added);
        }
    }

    /// Every cached page is misaligned, so the cache is dropped and refetched, but the model is not
    /// reset: `items_changed` at
    /// 0 is an insertion, so the selection follows its row and the view keeps its scroll anchor.
    fn inserted_at_top(&self, count: u32) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(move |list| list.inserted_at_top(count));
            return;
        }
        state.refresh.replace(None);
        let inserted = state.window.borrow_mut().inserted_at_top(count);
        if inserted {
            state.handed.borrow_mut().clear();
            items_changed(self, 0, 0, count);
        }
    }

    /// A call made while the model is answering `item()` is held and reports `false`. A message not
    /// on screen needs no
    /// update: its page is fetched fresh.
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
        // Same position and message: the row announces itself and the model stays quiet; only a row
        // nothing holds goes through
        // `items_changed`.
        let held = state.handed.borrow().get(&position).cloned();
        match held {
            Some(held) => held.fill(row),
            None => items_changed(self, position, 1, 1),
        }
        true
    }

    /// For when the order changed but the question did not, so a request in flight is not stale.
    fn invalidate(&self) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(|list| list.invalidate());
            return;
        }
        state.refresh.replace(None);
        let total = state.window.borrow_mut().invalidate();
        state.handed.borrow_mut().clear();
        items_changed(self, 0, total, total);
    }

    /// Re-reads the pages holding a row the view or selection holds; until all land the list
    /// answers with what it had. Each
    /// run is then compared by message id ([`postio_ui::list::splices`]) and only removals and
    /// insertions at their position
    /// are emitted, so the view keeps its scroll anchor and other rows' widgets. Asked again while
    /// one is out, it runs once
    /// more after it lands.
    fn refresh(&self) {
        let state = self.windowed();
        if state.reading.get() {
            self.hold(|list| list.refresh());
            return;
        }
        // A change-over's first page may predate whatever this refresh is for.
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
            // One reference is the map's own; more is the view's or the selection's row.
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
            let pending = {
                let mut window = state.window.borrow_mut();
                window.retain_pages(|_| false);
                window.is_pending(0)
            };
            state
                .handed
                .borrow_mut()
                .retain(|_, row| row.ref_count() > 1);
            if pending {
                // A read out since before what this refresh is for: mail written while the empty
                // inbox was being read stayed
                // unlisted (#2152).
                state.top_again.set(true);
            } else {
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

    fn all_resident(&self, messages: &[MessageId]) -> bool {
        let window = self.windowed().window.borrow();
        !messages.is_empty()
            && messages
                .iter()
                .all(|message| window.position_of(*message).is_some())
    }

    /// `false`, with nothing changed, unless every one is resident and removable; the caller
    /// reloads then. Removed
    /// bottom-up, one `items_changed(position, 1, 0)` each, so the view keeps every other row's
    /// widget.
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

    fn resident_rows(&self) -> usize {
        self.windowed().window.borrow().resident_rows()
    }

    fn page_of(&self, message: MessageId) -> Option<u32> {
        self.windowed().window.borrow().page_of(message)
    }

    fn note_pending(&self, page: u32) -> bool {
        self.windowed().window.borrow_mut().note_pending(page)
    }

    fn is_pending(&self, page: u32) -> bool {
        self.windowed().window.borrow().is_pending(page)
    }

    fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        self.windowed().window.borrow().pages_holding(messages)
    }

    /// `None` covers both "not in this mailbox" and "resident but not fetched yet"; callers treat
    /// them the same: ask for the
    /// page and retry.
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.windowed().window.borrow().position_of(message)
    }

    fn resident_pages(&self) -> Vec<u32> {
        self.windowed().window.borrow().resident_pages()
    }

    /// Unlike [`list_item`](Self::list_item), asks the source for nothing: a Shift-click across ten
    /// thousand rows must not
    /// request ten thousand.
    fn peek(&self, position: u32) -> Option<MessageId> {
        self.windowed().window.borrow().peek(position)
    }

    fn row_at(&self, position: u32) -> Option<Self::Row> {
        let state = self.windowed();
        // The same object as last time; `deliver_for` keeps its contents current.
        if let Some(row) = state.handed.borrow().get(&position) {
            return Some(row.clone());
        }

        // Mid-refresh or showing rows of a question being replaced: ask for nothing; the refresh or
        // change-over fills them.
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
            // Names which position provoked the request, which the reply-side log cannot (#1534).
            tracing::debug!(position, page, "list page wanted");
            request(self, page);
        }

        let handed = Self::Row::placeholder();
        if let Some(row) = row {
            handed.fill(row);
        }
        state.handed.borrow_mut().insert(position, handed.clone());
        Some(handed)
    }
}

fn items_changed<M: WindowedModel>(model: &M, position: u32, removed: u32, added: u32) {
    model
        .upcast_ref::<gtk::gio::ListModel>()
        .items_changed(position, removed, added);
}

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

/// Two steps: `GtkSingleSelection`, told its selected row was replaced, looks for it among every
/// added row with one `item()`
/// each (opening a 3,618-conversation folder asked for 73 pages). So the replace covers only the
/// rows already held from
/// the top and the rest is an insertion at the end.
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
    // Bottom up, so a run's steps never move a position a later run still uses.
    for (first, last) in refresh.runs.iter().rev() {
        if !(*first..=*last).all(|page| refresh.arrived.contains_key(&page)) {
            continue;
        }
        let start = first * PAGE_SIZE;
        let incoming: Vec<<M::Row as ModelRow>::Data> = (*first..=*last)
            .flat_map(|page| refresh.arrived.remove(&page).unwrap_or_default())
            .collect();
        let held: Vec<Option<postio_ui::list::RowKey>> = {
            let window = state.window.borrow();
            (start..(last + 1) * PAGE_SIZE)
                .map_while(|position| window.peek_key(position).map(Some))
                .collect()
        };
        let keys: Vec<postio_ui::list::RowKey> = incoming.iter().map(M::Row::key_of).collect();
        for step in postio_ui::list::splices(&held, &keys) {
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
        // A step the window could not take: it and the view disagree about a position, so start
        // over.
        model.emit_by_name::<()>("splicing", &[&false]);
        model.invalidate();
        request(model, 0);
        return;
    }

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

/// The one place a page is asked for, so the place to count from (#1534). Deduplication against
/// cached or pending pages
/// happens in [`ListWindow`].
fn request<M: WindowedModel>(model: &M, page: u32) {
    postio_ui::reader::cost::note_page_requested();
    let source = model.windowed().source.borrow().clone();
    if let Some(source) = source {
        source.request(page);
    }
}
