//! The message list's paging and generation bookkeeping — the model half,
//! shared by every frontend (ADR 0019 Q5a).
//!
//! # Where this ends and the toolkit begins
//!
//! [`ListWindow<T>`] owns the bookkeeping **and the resident rows**. What
//! stays with the toolkit — `postio-gtk`'s `MessageList`, and eventually
//! macOS's own thin wrapper — is row identity as *its* toolkit understands
//! it, change notification (`GListModel::items_changed`,
//! `NSTableView::reloadData(forRowIndexes:)`), and any re-entrancy rule a
//! toolkit's own contract imposes (GTK's `GListModel::item()` must not be
//! mutated mid-call; `NSTableView` has no such rule, so that guard is
//! `postio-gtk`'s alone to keep — see its own module docs).
//!
//! `ListScope` — which mailbox, or which smart folder — deliberately does
//! **not** move here either. `ListWindow` has no idea what a scope is; it
//! has [`reset`](ListWindow::reset), which bumps the generation and empties
//! the cache, and the feed calls it when the scope changes. A model that
//! knew about mailboxes and smart folders would be a second place deciding
//! what the list shows.
//!
//! # Why the rows move too
//!
//! The tempting smaller design is a `ListWindow` of *pure decisions* — it
//! tracks which pages are resident and answers "request these, evict that",
//! while each frontend keeps its own page storage. Rejected: it turns *"a
//! 100k-row scope never materialises more than the resident bound"*
//! (`PRODUCT.md` §18) into a claim about instructions issued, not about
//! memory held — and a second frontend can obey every instruction and still
//! hold the mailbox, because the thing that bounds memory is whoever owns
//! the map. Owning the rows makes the bound structural: one map, one
//! eviction, and [`resident_rows`](ListWindow::resident_rows) asserts
//! against the thing that actually holds them.
//!
//! # Why generic, and what [`ListRow`] is for
//!
//! A redelivered page must preserve identity for a row already resident —
//! on GTK that means the same `GObject`, so a flag change does not
//! invalidate anything holding onto it — and that behaviour must not be
//! re-derived by a second frontend. [`ListRow::reconcile`] carries it: the
//! default takes the incoming value, which is right for a plain value type,
//! and `postio-gtk` overrides it to update the existing object in place and
//! hand that back.
//!
//! # Every method returns what changed
//!
//! Nothing here emits anything — there is no callback, because a callback
//! shaped for `GListModel::items_changed` would not also be the call
//! `NSTableView` needs. Instead every mutating method answers with a small
//! value describing what changed, and the toolkit-side wrapper turns that
//! into whatever its own view needs to be told. `items_changed` and
//! `reloadData(forRowIndexes:)` become the same fact told to two views.
//!
//! # The one ordering rule
//!
//! **No method on [`ListWindow`] may be fallible, blocking, or async** —
//! `NSTableView`'s row callback runs on the main thread in microseconds and
//! must never `await`, and none of the methods here do. The corollary is
//! `postio-gtk`'s to keep, not this module's: `ListWindow` must never be
//! called from inside `GListModel::item()` while that call is still
//! answering, because a page source is free to answer synchronously and
//! this module has no way to know it is being asked from inside a read.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use postio_core::aim::{RowFacts, RowKind};
use postio_model::ids::{MessageId, ThreadId};

/// Rows per page.
///
/// Big enough that a screenful never spans more than two pages at any
/// sensible density, small enough that a page is a few kilobytes on the
/// wire from SQLite.
pub const PAGE_SIZE: u32 = 50;

/// Pages kept resident. Everything past this is evicted least-recently-used.
///
/// Eight pages is around 400 rows: roughly a screen either side of the
/// viewport at the airiest density, plus slack for a fast flick.
pub const CACHE_PAGES: usize = 8;

/// A row a [`ListWindow`] can hold.
///
/// The trait is the whole of what the model needs to know about a row: which
/// message it stands for, and what a redelivered copy of it should become.
pub trait ListRow {
    /// The message this row stands for, if it carries one.
    ///
    /// A row not yet loaded (a GTK placeholder, say) answers `None`; nothing
    /// in [`ListWindow`] ever constructs such a row itself, so this is a
    /// toolkit-side concern to expose, not one the model has to reason about.
    fn id(&self) -> Option<MessageId>;

    /// The conversation this row stands for, when it stands for one.
    ///
    /// `None` means "a message row": a query view, an unthreaded list, or a
    /// row that has not been loaded. That is the default because a plain
    /// value row carries no threading of its own — a list that has threads
    /// says so by overriding this.
    ///
    /// The one question `postio_core::aim` asks a frontend's list, reached
    /// through the blanket [`RowFacts`](postio_core::aim::RowFacts)
    /// implementation below. See that module for why the seam reports a fact
    /// and never a decision.
    fn thread(&self) -> Option<ThreadId> {
        None
    }

    /// A redelivered row for the same message.
    ///
    /// The default takes the incoming value, which is right for a plain
    /// value type — nothing in it needs to survive a replacement. GTK
    /// overrides this to update the existing `GObject` in place and hand
    /// that back, so anything holding the row keeps holding it.
    fn reconcile(_existing: &Self, incoming: Self) -> Self
    where
        Self: Sized,
    {
        incoming
    }
}

/// The answer to asking [`ListWindow`] for the row at a position.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<'a, T> {
    /// The row is here.
    Resident(&'a T),
    /// Not here. Draw a placeholder and issue these requests — the page
    /// itself and, at a boundary, its neighbour — deduplicated against
    /// whatever is already cached or already on its way.
    Missing {
        /// Pages to ask whatever backs the toolkit's page source for. Never
        /// a page already resident or already pending.
        request: Vec<u32>,
    },
}

/// What a page delivery changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// Dropped: a reply from a generation the user has already left. Nothing
    /// else on this value means anything when this is `true`.
    pub stale: bool,
    /// The positions the toolkit must tell its view about, if any rows in
    /// range actually landed.
    pub changed: Option<Range<u32>>,
    /// Pages evicted to stay inside [`CACHE_PAGES`]. Informational only —
    /// an evicted page is by definition one nothing is bound to any more, so
    /// nothing needs telling.
    pub evicted: Vec<u32>,
}

/// The paging and generation bookkeeping behind a windowed message list.
///
/// A plain struct with no interior mutability and no toolkit reference of
/// any kind — see the module docs for the reasoning behind the split.
pub struct ListWindow<T> {
    total: u32,
    /// Resident pages, by page index.
    pages: HashMap<u32, Vec<T>>,
    /// Page indices, least recently used first.
    recent: VecDeque<u32>,
    /// Pages already asked for and not yet delivered.
    pending: HashSet<u32>,
    /// Bumped by [`reset`](Self::reset). A reply from an older generation is
    /// answering a question nobody is asking any more.
    generation: u64,
}

impl<T> Default for ListWindow<T> {
    fn default() -> Self {
        ListWindow {
            total: 0,
            pages: HashMap::new(),
            recent: VecDeque::new(),
            pending: HashSet::new(),
            generation: 0,
        }
    }
}

impl<T: ListRow> ListWindow<T> {
    /// An empty window, generation zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many rows the current scope has, in total.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The generation in force. Stamp this on a request when it is made, and
    /// pass it back to [`deliver`](Self::deliver) when the answer arrives.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// How many rows are resident. The number the memory budget is about.
    pub fn resident_rows(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    /// Which pages are resident, lowest first. For tests and diagnostics.
    pub fn resident_pages(&self) -> Vec<u32> {
        let mut pages: Vec<u32> = self.pages.keys().copied().collect();
        pages.sort_unstable();
        pages
    }

    /// Point the window at a new scope: a different folder, a different
    /// search, a smart folder replacing a mailbox.
    ///
    /// Empties the cache and bumps the generation — a reply already in
    /// flight is answering a question this scope did not ask. Returns the
    /// new generation, for the caller to stamp on whatever request follows.
    /// Callers that need the row count `set_source` used to report as
    /// "removed" should read [`total`](Self::total) before calling this.
    pub fn reset(&mut self, total: u32) -> u64 {
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.total = total;
        self.generation += 1;
        self.generation
    }

    /// Drop everything cached and ask again, keeping both the scope and the
    /// generation.
    ///
    /// For when the order itself moved — a resync, a re-sort — and the rows
    /// answering are still answering the same question, so a request already
    /// in flight from before this call is not stale. Returns the row count
    /// so the caller can report every position as changed.
    pub fn invalidate(&mut self) -> u32 {
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.total
    }

    /// Correct the row count without touching what is cached, beyond
    /// forgetting what now lies past the end.
    ///
    /// For a total that shrank or grew at the end of the scope. New rows
    /// arriving at the front are [`inserted_at_top`](Self::inserted_at_top).
    /// Returns `(position, removed, added)` — the `items_changed`-shaped
    /// triple — or `None` if the total did not actually change.
    pub fn set_total(&mut self, total: u32) -> Option<(u32, u32, u32)> {
        let previous = self.total;
        if previous == total {
            return None;
        }
        self.total = total;
        if total > previous {
            Some((previous, 0, total - previous))
        } else {
            self.drop_pages_from(total);
            Some((total, previous - total, 0))
        }
    }

    /// New rows landed at the front of the scope.
    ///
    /// Every row shifts down by `count`, which misaligns every cached page
    /// against its positions, so the cache is dropped — costing a refetch,
    /// which is cheaper than serving a misaligned one. The generation is
    /// **not** bumped: this is still the same scope, just longer. Returns
    /// whether anything actually happened; `count == 0` is a no-op the
    /// caller need not report.
    pub fn inserted_at_top(&mut self, count: u32) -> bool {
        if count == 0 {
            return false;
        }
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.total += count;
        true
    }

    /// The row at `position`, fetching its page — and, at a boundary, the
    /// page either side — if it is not resident.
    ///
    /// `None` for a position outside the current total.
    pub fn row_at(&mut self, position: u32) -> Option<Lookup<'_, T>> {
        if position >= self.total {
            return None;
        }
        let page = position / PAGE_SIZE;
        let index = (position % PAGE_SIZE) as usize;

        if self.pages.get(&page).is_some_and(|rows| index < rows.len()) {
            self.touch(page);
            return Some(Lookup::Resident(&self.pages[&page][index]));
        }

        // Not here: ask for it, and for the pages either side, so scrolling
        // at speed does not stall on a page boundary.
        let mut request = Vec::with_capacity(3);
        self.want(page, &mut request);
        if page > 0 {
            self.want(page - 1, &mut request);
        }
        self.want(page + 1, &mut request);
        Some(Lookup::Missing { request })
    }

    /// Mark `page` as worth a fresh request, and note it in `into` — unless
    /// it is already resident or already on its way.
    fn want(&mut self, page: u32, into: &mut Vec<u32>) {
        if page * PAGE_SIZE >= self.total {
            return;
        }
        if self.pages.contains_key(&page) || !self.pending.insert(page) {
            return;
        }
        into.push(page);
    }

    /// Accept a page of rows delivered for `generation`.
    ///
    /// Rows already resident for the same message are reconciled through
    /// [`ListRow::reconcile`] rather than replaced outright, so a
    /// redelivered page does not invalidate anything holding onto them.
    pub fn deliver(&mut self, generation: u64, page: u32, rows: Vec<T>) -> Delivered {
        if generation != self.generation {
            return Delivered {
                stale: true,
                changed: None,
                evicted: Vec::new(),
            };
        }
        self.pending.remove(&page);

        let existing: HashMap<MessageId, T> = self
            .pages
            .remove(&page)
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row.id()?, row)))
            .collect();

        let count = rows.len() as u32;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let reconciled = match row.id().and_then(|id| existing.get(&id)) {
                Some(old) => T::reconcile(old, row),
                None => row,
            };
            items.push(reconciled);
        }

        self.pages.insert(page, items);
        self.touch(page);
        let evicted = self.evict();

        // The positions did not move; what they answer with did.
        let start = page * PAGE_SIZE;
        let span = count.min(self.total.saturating_sub(start));
        let changed = (span > 0).then_some(start..start + span);

        Delivered {
            stale: false,
            changed,
            evicted,
        }
    }

    /// Update one row in place — a flag, read state, a label — without
    /// touching its position or its page's eviction standing.
    ///
    /// Reconciled through [`ListRow::reconcile`], the same as a redelivered
    /// page, so the row keeps its identity. Returns the row's position if it
    /// was resident; a row that is not on screen needs no update, because
    /// its page is read fresh when it next is.
    pub fn update(&mut self, incoming: T) -> Option<u32> {
        let id = incoming.id()?;
        let found = self.pages.iter().find_map(|(page, rows)| {
            let index = rows.iter().position(|row| row.id() == Some(id))?;
            Some((*page, index))
        })?;
        let (page, index) = found;
        let rows = self.pages.get_mut(&page).expect("just found it");
        let updated = T::reconcile(&rows[index], incoming);
        rows[index] = updated;
        Some(page * PAGE_SIZE + index as u32)
    }

    /// The message at `position`, but only if its page is already resident.
    ///
    /// [`row_at`](Self::row_at) fetches what it does not have, which is
    /// right for drawing a row that has to become real — and wrong for
    /// answering "what is in this range". A Shift-click across ten thousand
    /// rows must not ask the store for ten thousand rows: the ones scrolled
    /// through are resident and get selected, the ones jumped over were
    /// never on screen.
    pub fn peek(&self, position: u32) -> Option<MessageId> {
        if position >= self.total {
            return None;
        }
        self.pages
            .get(&(position / PAGE_SIZE))
            .and_then(|rows| rows.get((position % PAGE_SIZE) as usize))
            .and_then(T::id)
    }

    /// Where `message` sits, among the rows currently resident.
    ///
    /// `None` covers both "not in this scope" and "resident scope, but this
    /// row not fetched yet" — a caller that wants to put the cursor on a
    /// message it did not just deliver itself cannot tell those apart and
    /// has to treat them the same: ask for the page, and try again once it
    /// answers.
    pub fn position_of(&self, message: MessageId) -> Option<u32> {
        self.pages.iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.id() == Some(message))
                .map(|index| page * PAGE_SIZE + index as u32)
        })
    }

    /// The resident row for `message`, if the window still holds one.
    ///
    /// Resident-only, and that is the whole contract: a row that has been
    /// paged out answers `None` rather than being fetched, because the
    /// callers of this are answering a question about what the user can see
    /// and a fetch would make a keystroke wait on the store.
    pub fn row_of(&self, message: MessageId) -> Option<&T> {
        self.pages
            .values()
            .find_map(|rows| rows.iter().find(|row| row.id() == Some(message)))
    }

    /// Which resident page holds `message`, if any.
    ///
    /// The cheap half of reacting to a change: a message that changed
    /// somewhere off screen needs nothing done, and one on screen costs a
    /// refetch of its page rather than of the whole scope.
    pub fn page_of(&self, message: MessageId) -> Option<u32> {
        self.position_of(message)
            .map(|position| position / PAGE_SIZE)
    }

    /// Every resident page holding any of `messages`, deduplicated.
    ///
    /// The bulk form of [`page_of`](Self::page_of), for a burst of changes
    /// that land together — a resync's `MessagesChanged`, say — so the
    /// caller issues one request per affected page rather than one per
    /// message.
    pub fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        let mut pages: Vec<u32> = messages
            .iter()
            .filter_map(|message| self.page_of(*message))
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }

    /// Mark `page` as the most recently used.
    fn touch(&mut self, page: u32) {
        self.recent.retain(|p| *p != page);
        self.recent.push_back(page);
    }

    /// Drop the least recently used pages down to [`CACHE_PAGES`], reporting
    /// which ones went.
    fn evict(&mut self) -> Vec<u32> {
        let mut evicted = Vec::new();
        while self.recent.len() > CACHE_PAGES {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            self.pages.remove(&oldest);
            evicted.push(oldest);
        }
        evicted
    }

    /// Forget every cached page that lies wholly past `position`.
    fn drop_pages_from(&mut self, position: u32) {
        let first_stale = position.div_ceil(PAGE_SIZE);
        self.pages.retain(|page, _| *page < first_stale);
        self.recent.retain(|page| *page < first_stale);
        self.pending.retain(|page| *page < first_stale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row whose id is its position, so a test can say where it came from.
    /// `Clone`/`PartialEq` only for the tests below; `ListWindow` itself
    /// needs neither.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Fixture {
        id: MessageId,
        reconciled: bool,
    }

    fn row(position: u32) -> Fixture {
        Fixture {
            id: MessageId::new(position as i64 + 1),
            reconciled: false,
        }
    }

    fn page_rows(page: u32, total: u32) -> Vec<Fixture> {
        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(total);
        (start..end).map(row).collect()
    }

    impl ListRow for Fixture {
        fn id(&self) -> Option<MessageId> {
            Some(self.id)
        }

        fn reconcile(existing: &Self, incoming: Self) -> Self {
            let _ = existing;
            Fixture {
                reconciled: true,
                ..incoming
            }
        }
    }

    /// A window with the current generation already stamped on `page`'s
    /// delivery, for a test that does not care about staleness.
    fn deliver_fresh(window: &mut ListWindow<Fixture>, page: u32, total: u32) -> Delivered {
        window.deliver(window.generation(), page, page_rows(page, total))
    }

    #[test]
    fn a_hundred_thousand_row_scope_never_materialises_more_than_the_resident_bound() {
        const HUGE: u32 = 100_000;
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(HUGE);

        let ceiling = CACHE_PAGES * PAGE_SIZE as usize;
        let mut position = 0;
        while position < HUGE {
            for offset in 0..15u32 {
                let at = (position + offset).min(HUGE - 1);
                if let Some(Lookup::Missing { request }) = window.row_at(at) {
                    for page in request {
                        deliver_fresh(&mut window, page, HUGE);
                    }
                }
            }
            assert!(
                window.resident_rows() <= ceiling,
                "at position {position} the window held {} rows, over the {ceiling} \
                 the cache is allowed",
                window.resident_rows()
            );
            position += 500;
        }
        assert_eq!(window.total(), HUGE);
    }

    #[test]
    fn a_jump_to_row_ninety_thousand_asks_for_its_page_once_each() {
        // A cold jump costs its own page plus a neighbour either side, so
        // scrolling on from here does not stall on a page boundary — never
        // the whole 100k-row scope, and never the same page twice even
        // though three requests land in the same call.
        const HUGE: u32 = 100_000;
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(HUGE);

        let Some(Lookup::Missing { request }) = window.row_at(90_000) else {
            panic!("row 90,000 cannot be resident in a fresh window");
        };
        assert_eq!(
            request,
            vec![1800, 1799, 1801],
            "the page itself, then the one before it, then the one after — \
             each named once"
        );

        // And once that answers, asking again for the same row costs nothing.
        deliver_fresh(&mut window, 1800, HUGE);
        deliver_fresh(&mut window, 1799, HUGE);
        deliver_fresh(&mut window, 1801, HUGE);
        assert!(matches!(window.row_at(90_000), Some(Lookup::Resident(_))));
    }

    #[test]
    fn a_page_is_never_asked_for_twice_while_outstanding() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);

        let Some(Lookup::Missing { request }) = window.row_at(0) else {
            panic!("position 0 cannot be resident yet");
        };
        assert_eq!(request, vec![0, 1]);

        // Still outstanding: asking again must not request it a second time.
        for position in 0..PAGE_SIZE {
            window.row_at(position);
        }
        assert!(matches!(
            window.row_at(PAGE_SIZE - 1),
            Some(Lookup::Missing { request }) if request.is_empty()
        ));
    }

    #[test]
    fn a_superseded_generations_reply_is_dropped() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        let old_generation = window.reset(1_000);
        window.row_at(0);

        // The scope changed before the answer came back.
        window.reset(500);

        let delivered = window.deliver(old_generation, 0, page_rows(0, 1_000));
        assert!(delivered.stale, "an old generation's reply must be dropped");
        assert_eq!(
            window.resident_rows(),
            0,
            "the stale reply must not have written anything"
        );
    }

    #[test]
    fn insert_at_top_keeps_the_cursor_on_its_row() {
        // "Keeping the cursor on its row" is what an insertion at position 0
        // means to a selection model one layer up: the row does not move in
        // the underlying data, so once the store confirms three new messages
        // landed ahead of it, the same message is found three positions
        // further down.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(1_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 1_000);
        let cursor_id = window.peek(7).expect("resident before the insert");
        assert_eq!(cursor_id, MessageId::new(8));

        assert!(window.inserted_at_top(3));
        assert_eq!(window.total(), 1_003);
        assert_eq!(
            window.resident_rows(),
            0,
            "every cached page is now misaligned against its positions"
        );

        window.row_at(10);
        // The store's own answer once re-asked: three new messages ahead of
        // everything that was already there, so what was at position 7 is
        // now at 10 — a fixture-only stand-in for what a real page source
        // would report, since nothing here simulates a live mailbox.
        let shifted: Vec<Fixture> = (0..PAGE_SIZE)
            .map(|position| {
                let id = if position < 3 {
                    100_000 + position as i64
                } else {
                    position as i64 - 3 + 1
                };
                Fixture {
                    id: MessageId::new(id),
                    reconciled: false,
                }
            })
            .collect();
        window.deliver(window.generation(), 0, shifted);

        assert_eq!(
            window.peek(10),
            Some(cursor_id),
            "the same message now sits three rows further down"
        );
    }

    #[test]
    fn inserting_nothing_changes_nothing() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(1_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 1_000);
        let resident = window.resident_rows();

        assert!(!window.inserted_at_top(0));
        assert_eq!(window.resident_rows(), resident);
    }

    #[test]
    fn the_pages_that_go_are_the_ones_nobody_is_looking_at() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);

        for page in 0..(CACHE_PAGES as u32 * 3) {
            window.row_at(page * PAGE_SIZE);
            deliver_fresh(&mut window, page, 100_000);
        }

        let resident = window.resident_pages();
        assert!(resident.len() <= CACHE_PAGES);
        assert!(
            !resident.contains(&0),
            "the first page is the least recently used and should be long gone"
        );
        assert!(resident.contains(&(CACHE_PAGES as u32 * 3 - 1)));
    }

    #[test]
    fn a_redelivered_page_is_reconciled_not_replaced() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 100_000);
        assert_eq!(window.peek(3), Some(MessageId::new(4)), "resident already");

        deliver_fresh(&mut window, 0, 100_000);
        let Some(Lookup::Resident(row)) = window.row_at(3) else {
            panic!("row 3 should be resident");
        };
        assert!(
            row.reconciled,
            "a redelivered row goes through reconcile, not a fresh insert"
        );
    }

    #[test]
    fn a_flag_change_touches_one_row_and_reports_its_position() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 100_000);

        let position = window.update(row(7));
        assert_eq!(position, Some(7));
        let Some(Lookup::Resident(updated)) = window.row_at(7) else {
            panic!("row 7 should still be resident");
        };
        assert!(updated.reconciled);
    }

    #[test]
    fn a_message_off_screen_needs_no_update() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 100_000);

        assert_eq!(
            window.update(row(90_000)),
            None,
            "its page is not resident, so there is nothing to update"
        );
    }

    #[test]
    fn a_shrinking_scope_drops_the_rows_that_went() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);
        for page in 0..10 {
            window.row_at(page * PAGE_SIZE);
            deliver_fresh(&mut window, page, 500);
        }
        assert!(window.resident_pages().contains(&7));

        let change = window.set_total(120);
        assert_eq!(change, Some((120, 380, 0)));
        assert!(window.resident_pages().iter().all(|page| *page < 3));
        assert_eq!(window.peek(120), None, "nothing past the new end");
    }

    #[test]
    fn a_growing_scope_reports_only_the_new_rows() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        assert_eq!(window.set_total(500), Some((120, 0, 380)));
    }

    #[test]
    fn an_unchanged_total_reports_nothing() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        assert_eq!(window.set_total(120), None);
    }

    #[test]
    fn resetting_replaces_the_scope_and_bumps_the_generation() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        let first = window.reset(1_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 1_000);
        assert!(window.resident_rows() > 0);

        let second = window.reset(40);
        assert_eq!(second, first + 1);
        assert_eq!(window.total(), 40);
        assert_eq!(window.resident_rows(), 0);
    }

    #[test]
    fn invalidating_keeps_the_scope_and_the_generation() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        let generation = window.reset(500);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 500);

        let total = window.invalidate();
        assert_eq!(total, 500, "the scope is the same length");
        assert_eq!(window.resident_rows(), 0);
        assert_eq!(
            window.generation(),
            generation,
            "a reorder is still the same question, so a request already in \
             flight from before it must not be treated as stale"
        );
    }

    #[test]
    fn there_is_nothing_past_the_end() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        assert!(window.row_at(120).is_none());
        assert!(window.row_at(u32::MAX).is_none());

        // And the window does not ask for a page that lies wholly past the
        // end: only the previous page joins the one actually requested.
        let Some(Lookup::Missing { request }) = window.row_at(119) else {
            panic!("row 119 should need a fetch");
        };
        assert_eq!(request, vec![2, 1], "no page 3, which starts past 120");
    }

    #[test]
    fn the_window_can_say_which_page_holds_a_message() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        window.row_at(PAGE_SIZE * 4);
        deliver_fresh(&mut window, 0, 100_000);
        deliver_fresh(&mut window, 4, 100_000);

        assert_eq!(window.page_of(MessageId::new(1)), Some(0));
        assert_eq!(
            window.page_of(MessageId::new(PAGE_SIZE as i64 * 4 + 1)),
            Some(4)
        );
        assert_eq!(
            window.page_of(MessageId::new(PAGE_SIZE as i64 * 2 + 1)),
            None,
            "a message whose page is not resident has no page to refetch"
        );
    }

    #[test]
    fn pages_holding_dedupes_and_sorts() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        window.row_at(PAGE_SIZE * 4);
        deliver_fresh(&mut window, 0, 100_000);
        deliver_fresh(&mut window, 4, 100_000);

        let ids = [
            MessageId::new(3),
            MessageId::new(PAGE_SIZE as i64 * 4 + 1),
            MessageId::new(1),
            MessageId::new(999_999), // resident nowhere
        ];
        assert_eq!(window.pages_holding(&ids), vec![0, 4]);
    }

    #[test]
    fn the_window_can_say_where_a_resident_message_sits() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        window.row_at(0);
        deliver_fresh(&mut window, 0, 100_000);

        assert_eq!(window.position_of(MessageId::new(1)), Some(0));
        assert_eq!(
            window.position_of(MessageId::new(7)),
            Some(6),
            "position, not just page — the row's own offset within it"
        );
        assert_eq!(
            window.position_of(MessageId::new(PAGE_SIZE as i64 * 2 + 1)),
            None
        );
    }

    #[test]
    fn delivering_evicts_when_the_cache_overflows() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(100_000);
        for page in 0..CACHE_PAGES as u32 {
            window.row_at(page * PAGE_SIZE);
            let delivered = deliver_fresh(&mut window, page, 100_000);
            assert!(delivered.evicted.is_empty(), "cache not yet full");
        }
        window.row_at(CACHE_PAGES as u32 * PAGE_SIZE);
        let delivered = deliver_fresh(&mut window, CACHE_PAGES as u32, 100_000);
        assert_eq!(
            delivered.evicted,
            vec![0],
            "the least recently used page goes"
        );
    }

    #[test]
    fn a_default_row_type_replaces_rather_than_reconciles() {
        // The Swift-facing default: `reconcile` just takes the incoming
        // value, which is correct for a plain value type with no identity
        // of its own to preserve.
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Plain(MessageId);
        impl ListRow for Plain {
            fn id(&self) -> Option<MessageId> {
                Some(self.0)
            }
        }

        let mut window: ListWindow<Plain> = ListWindow::new();
        window.reset(10);
        window.row_at(0);
        window.deliver(window.generation(), 0, vec![Plain(MessageId::new(1))]);
        let updated = window.update(Plain(MessageId::new(1)));
        assert_eq!(updated, Some(0));
    }
}

/// Every [`ListWindow`] is a source of row facts for `postio_core::aim`.
///
/// Blanket, so neither frontend writes one: GTK drives a
/// `ListWindow<MessageRow>` and the FFI boundary a `ListWindow<RowFfi>`, and
/// both get the same answer to the same question by construction. The
/// `Missing` arm is not an error case — it is what a marked row that has been
/// paged out has to report, so that the shared rules can decline to guess
/// what it was (#468).
impl<T: ListRow> RowFacts for ListWindow<T> {
    fn row_kind(&self, message: MessageId) -> RowKind {
        match self.row_of(message) {
            None => RowKind::Missing,
            Some(row) => match row.thread() {
                Some(thread) => RowKind::Thread(thread),
                None => RowKind::Message,
            },
        }
    }
}

#[cfg(test)]
mod row_facts_tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Row {
        id: MessageId,
        thread: Option<ThreadId>,
    }

    impl ListRow for Row {
        fn id(&self) -> Option<MessageId> {
            Some(self.id)
        }

        fn thread(&self) -> Option<ThreadId> {
            self.thread
        }
    }

    fn window_holding(rows: Vec<Row>) -> ListWindow<Row> {
        let mut window: ListWindow<Row> = ListWindow::new();
        window.reset(rows.len() as u32);
        window.row_at(0);
        let generation = window.generation();
        window.deliver(generation, 0, rows);
        window
    }

    #[test]
    fn a_resident_conversation_row_reports_its_thread() {
        let window = window_holding(vec![Row {
            id: MessageId::new(7),
            thread: Some(ThreadId::new(3)),
        }]);

        assert_eq!(
            window.row_kind(MessageId::new(7)),
            RowKind::Thread(ThreadId::new(3)),
        );
    }

    #[test]
    fn a_resident_message_row_reports_a_message() {
        let window = window_holding(vec![Row {
            id: MessageId::new(7),
            thread: None,
        }]);

        assert_eq!(window.row_kind(MessageId::new(7)), RowKind::Message);
    }

    /// The arm #468 turns on: a marked row that has been paged out cannot be
    /// checked, and the shared rules decline to guess rather than acting on
    /// a conversation the user may never have marked.
    #[test]
    fn a_row_the_window_does_not_hold_is_missing_rather_than_a_message() {
        let window = window_holding(vec![Row {
            id: MessageId::new(7),
            thread: Some(ThreadId::new(3)),
        }]);

        assert_eq!(window.row_kind(MessageId::new(99)), RowKind::Missing);
    }

    /// `row_of` must not reach past what is resident: answering from the
    /// store would make a keystroke wait on it, and a mailbox is never
    /// materialised (`PRODUCT.md` §18).
    #[test]
    fn an_empty_window_holds_no_rows_rather_than_fetching_any() {
        let window: ListWindow<Row> = ListWindow::new();
        assert_eq!(window.row_kind(MessageId::new(7)), RowKind::Missing);
        assert!(window.row_of(MessageId::new(7)).is_none());
    }
}
