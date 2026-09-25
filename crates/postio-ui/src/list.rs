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
    /// through the blanket [`RowFacts`]
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
    /// Every row landed on top of a row that was already resident, in the
    /// same order and the same number: the same messages, with new contents.
    ///
    /// The distinction matters because it is the difference between "these
    /// positions answer with different rows now" and "these rows say
    /// something different now", and a toolkit list answers the first by
    /// throwing away the widgets in range and building them again. Reading
    /// one message refetches its page, and every row in that page comes back
    /// identical but for one flag — so announcing it as a replacement makes
    /// the whole visible list blink for a flag on one row. A row that can
    /// announce its own change needs no such announcement, and `changed`
    /// still carries the range for whoever cannot.
    pub reconciled: bool,
}

/// One step of turning the rows a stretch of the list held into the rows it
/// holds now. See [`splices`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Splice {
    /// Take `count` rows out at `at`.
    Remove {
        /// Where, counted from the start of the stretch, in the state the
        /// steps before this one left.
        at: u32,
        /// How many.
        count: u32,
    },
    /// Put `count` rows in at `at`: the new stretch's rows `from..from + count`.
    Insert {
        /// Where, counted from the start of the stretch, in the state the
        /// steps before this one left.
        at: u32,
        /// How many.
        count: u32,
        /// Where they start in the new stretch.
        from: usize,
    },
}

/// The fewest positional steps that turn `old` into `new`: removals first,
/// bottom up, then insertions, top down, each position counted in the state
/// the steps before it left.
///
/// This is what lets a list re-read a stretch it is showing and tell its
/// view only what moved. A row whose message is in both, in the same order
/// relative to the others that stay, stays -- the same object, the same
/// widget, updated in place by whoever applies this -- and every other row
/// is an insertion or a removal *at its position*. Announcing the stretch as
/// replaced instead is what rebuilt every widget in it and let the list
/// flash skeletons on a resync (maintainer, 2026-09-25).
///
/// Which rows stay is the longest run of shared messages whose order did not
/// change, so a conversation that moved to the top is one removal and one
/// insertion, and mail landing above a page pushes one row off its end
/// rather than shifting fifty. A position `old` knew nothing about (`None`,
/// a placeholder) is always replaced.
pub fn splices(old: &[Option<MessageId>], new: &[MessageId]) -> Vec<Splice> {
    let mut held: HashMap<MessageId, usize> = HashMap::with_capacity(old.len());
    for (index, id) in old.iter().enumerate() {
        if let Some(id) = id {
            held.entry(*id).or_insert(index);
        }
    }
    let shared: Vec<(usize, usize)> = new
        .iter()
        .enumerate()
        .filter_map(|(to, id)| held.get(id).map(|from| (*from, to)))
        .collect();
    let mut keep_old = vec![false; old.len()];
    let mut keep_new = vec![false; new.len()];
    for (index, kept) in longest_in_order(&shared).into_iter().enumerate() {
        if kept {
            let (from, to) = shared[index];
            keep_old[from] = true;
            keep_new[to] = true;
        }
    }

    let mut script = Vec::new();
    let mut end = old.len();
    while end > 0 {
        if keep_old[end - 1] {
            end -= 1;
            continue;
        }
        let mut start = end;
        while start > 0 && !keep_old[start - 1] {
            start -= 1;
        }
        script.push(Splice::Remove {
            at: start as u32,
            count: (end - start) as u32,
        });
        end = start;
    }
    let mut start = 0;
    while start < new.len() {
        if keep_new[start] {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < new.len() && !keep_new[end] {
            end += 1;
        }
        script.push(Splice::Insert {
            at: start as u32,
            count: (end - start) as u32,
            from: start,
        });
        start = end;
    }
    script
}

/// Which of `pairs` -- `(old position, new position)`, in new order -- form
/// the longest run whose old positions also increase. Patience sorting, so a
/// stretch of a few hundred rows costs nothing worth measuring.
fn longest_in_order(pairs: &[(usize, usize)]) -> Vec<bool> {
    let mut tails: Vec<usize> = Vec::new();
    let mut before: Vec<Option<usize>> = vec![None; pairs.len()];
    for (index, (from, _)) in pairs.iter().enumerate() {
        let slot = tails.partition_point(|tail| pairs[*tail].0 < *from);
        if slot > 0 {
            before[index] = Some(tails[slot - 1]);
        }
        if slot == tails.len() {
            tails.push(index);
        } else {
            tails[slot] = index;
        }
    }
    let mut kept = vec![false; pairs.len()];
    let mut at = tails.last().copied();
    while let Some(index) = at {
        kept[index] = true;
        at = before[index];
    }
    kept
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
    /// Replies to drop when they land, per page: asked for at offsets a
    /// [`splice`](Self::splice) has since moved, so applying one would put
    /// every row in it one place out.
    discard: HashMap<u32, u32>,
}

impl<T> Default for ListWindow<T> {
    fn default() -> Self {
        ListWindow {
            total: 0,
            pages: HashMap::new(),
            recent: VecDeque::new(),
            pending: HashSet::new(),
            generation: 0,
            discard: HashMap::new(),
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
        let next = self.generation + 1;
        self.adopt(total, next);
        next
    }

    /// [`reset`](Self::reset) to a generation the caller already handed out:
    /// a list that keeps showing the scope it is leaving until the new one's
    /// first page lands stamps that page's request with the generation it
    /// will be answered under, and adopts it when the answer arrives.
    pub fn adopt(&mut self, total: u32, generation: u64) {
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.discard.clear();
        self.total = total;
        self.generation = generation;
    }

    /// Move to `generation` keeping every held page: the rows are still
    /// right, but nothing asked for under the old generation may land on
    /// them. What a list does when it puts back a scope it had set aside,
    /// rows and all, rather than reading it again.
    pub fn renumber(&mut self, generation: u64) {
        self.pending.clear();
        self.discard.clear();
        self.generation = generation;
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
        self.discard.clear();
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

    /// Remove the row at `position`, shifting every row after it up by one.
    ///
    /// `false`, with nothing changed, when the row is not resident or a page
    /// from here on is still on its way -- a page asked for at the old
    /// offsets would land one row out -- and the caller reloads instead.
    pub fn remove_at(&mut self, position: u32) -> bool {
        if position >= self.total {
            return false;
        }
        let page = position / PAGE_SIZE;
        let index = (position % PAGE_SIZE) as usize;
        if self.pending.iter().any(|pending| *pending >= page)
            || !self.pages.get(&page).is_some_and(|rows| index < rows.len())
        {
            return false;
        }
        self.pages
            .get_mut(&page)
            .expect("checked above")
            .remove(index);
        self.total -= 1;

        // Each following page that is held gives its first row to the one
        // before it, for as long as the held pages run on unbroken. The last
        // of them is then one row short, which `row_at` already treats as a
        // page to ask for again when that row is wanted (#1165).
        let mut last = page;
        while let Some(next) = self.pages.get_mut(&(last + 1)) {
            if next.is_empty() {
                break;
            }
            let carried = next.remove(0);
            self.pages
                .get_mut(&last)
                .expect("the run is held")
                .push(carried);
            last += 1;
        }
        // Past a page nobody holds, every row would sit one position out.
        self.pages.retain(|held, _| *held <= last);
        self.recent.retain(|held| *held <= last);
        true
    }

    /// Replace `removed` rows at `position` with `inserted`, inside the run
    /// of held pages that holds it -- one [`Splice`] step, applied.
    ///
    /// A run is held pages back to back, every one of them full but the
    /// last, so its rows are exactly the positions it covers. The run is
    /// re-cut into pages after the change: a row pushed off the end of one
    /// page is the first of the next, and a run that grew past its last full
    /// page ends on a new short one, which [`row_at`](Self::row_at) already
    /// treats as a page to ask for again once a row past it is wanted.
    ///
    /// When the run's length changes, everything after it is one row out per
    /// row gained or lost: pages held past it are dropped, and a page asked
    /// for at the old offsets has its answer dropped when it lands -- it is
    /// askable again from now. `false`, with nothing changed, when no held
    /// run covers the change; the end of a run counts, for a stretch that
    /// grew there.
    pub fn splice(&mut self, position: u32, removed: u32, inserted: Vec<T>) -> bool {
        if position.saturating_add(removed) > self.total {
            return false;
        }
        let full = |pages: &HashMap<u32, Vec<T>>, page: u32| {
            pages
                .get(&page)
                .is_some_and(|rows| rows.len() == PAGE_SIZE as usize)
        };
        let page = position / PAGE_SIZE;
        let index = (position % PAGE_SIZE) as usize;
        let anchor = if self
            .pages
            .get(&page)
            .is_some_and(|rows| index <= rows.len())
        {
            page
        } else if index == 0 && page > 0 && full(&self.pages, page - 1) {
            page - 1
        } else {
            return false;
        };
        let mut first = anchor;
        while first > 0 && full(&self.pages, first - 1) {
            first -= 1;
        }
        let mut last = anchor;
        while full(&self.pages, last) && self.pages.contains_key(&(last + 1)) {
            last += 1;
        }
        let start = first * PAGE_SIZE;
        let mut rows: Vec<T> = Vec::new();
        for held in first..=last {
            rows.extend(self.pages.remove(&held).unwrap_or_default());
        }
        let offset = (position - start) as usize;
        if offset + removed as usize > rows.len() {
            // The change reaches past what the run holds. Put it back as it
            // was and refuse.
            self.rechunk(first, rows);
            return false;
        }
        let before = rows.len();
        let added = inserted.len() as u32;
        rows.splice(offset..offset + removed as usize, inserted);
        let grew = rows.len() != before;
        let after_run = first + (rows.len() as u32).div_ceil(PAGE_SIZE).max(1);
        self.rechunk(first, rows);
        self.total = self.total - removed + added;
        if grew {
            self.pages.retain(|held, _| *held < after_run);
            let moved: Vec<u32> = self
                .pending
                .iter()
                .copied()
                .filter(|pending| *pending >= first)
                .collect();
            for pending in moved {
                self.pending.remove(&pending);
                *self.discard.entry(pending).or_insert(0) += 1;
            }
        }
        self.recent.retain(|held| self.pages.contains_key(held));
        let mut touched: Vec<u32> = (first..after_run)
            .filter(|held| self.pages.contains_key(held))
            .collect();
        touched.retain(|held| !self.recent.contains(held));
        self.recent.extend(touched);
        true
    }

    /// Put `rows` back as pages from `first` on, the last of them short if
    /// it has to be.
    fn rechunk(&mut self, first: u32, rows: Vec<T>) {
        let mut page = first;
        let mut rest = rows.into_iter().peekable();
        while rest.peek().is_some() {
            let chunk: Vec<T> = rest.by_ref().take(PAGE_SIZE as usize).collect();
            self.pages.insert(page, chunk);
            page += 1;
        }
    }

    /// Forget every held page `keep` says no to, asking for nothing.
    ///
    /// For a refresh that re-read only the pages somebody is looking at: the
    /// rest are one row out for every row the refresh moved, so they go, and
    /// are read again at their new offsets when they are next wanted.
    pub fn retain_pages(&mut self, keep: impl Fn(u32) -> bool) {
        self.pages.retain(|page, _| keep(*page));
        self.recent.retain(|page| keep(*page));
    }

    /// Say that the answer for `page` has been taken, without delivering it
    /// here -- a refresh holds its pages until they have all landed, then
    /// applies them as splices.
    pub fn answered(&mut self, generation: u64, page: u32) -> bool {
        if generation != self.generation {
            return false;
        }
        if let Some(count) = self.discard.get_mut(&page)
            && *count > 0
        {
            *count -= 1;
            return false;
        }
        self.pending.remove(&page);
        true
    }

    /// Every row held, in no particular order, asking for nothing.
    ///
    /// For a caller that wants what is already here -- the subset of a
    /// conversation the list holds -- where [`row_at`](Self::row_at) over
    /// every position would ask for every page it does not hold.
    pub fn resident(&self) -> impl Iterator<Item = &T> {
        self.pages.values().flatten()
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
        // at speed does not stall on a page boundary. The page actually
        // being read must be long enough to cover `index`, not merely
        // present — a page can be resident and still too short for the
        // position asked of it (#1165). The neighbours are pure prefetch
        // with no position of their own to answer for, so presence alone is
        // still the right question there.
        let mut request = Vec::with_capacity(3);
        self.want(page, Some(index), &mut request);
        if page > 0 {
            self.want(page - 1, None, &mut request);
        }
        self.want(page + 1, None, &mut request);
        Some(Lookup::Missing { request })
    }

    /// Mark `page` as worth a fresh request, and note it in `into` — unless
    /// it is already resident or already on its way.
    ///
    /// `required_index`, when given, is the row within `page` that must
    /// actually be present for the page to count as resident: `row_at`'s own
    /// `index < rows.len()` question, asked here too. Without it, a page
    /// delivered short of `PAGE_SIZE` — the last page legitimately, or a
    /// middle page whose fetch raced a shrinking store — reads as resident
    /// forever once `pages` merely contains its key, and the position it
    /// cannot answer for is stuck showing a placeholder with nothing left to
    /// ever ask for it again. `None` is the coarser question a neighbour
    /// prefetched for its own sake, rather than a position anyone asked for,
    /// still wants: present at all, whatever its length.
    fn want(&mut self, page: u32, required_index: Option<usize>, into: &mut Vec<u32>) {
        if page * PAGE_SIZE >= self.total {
            return;
        }
        let resident = match required_index {
            Some(index) => self.pages.get(&page).is_some_and(|rows| index < rows.len()),
            None => self.pages.contains_key(&page),
        };
        if resident || !self.pending.insert(page) {
            return;
        }
        into.push(page);
    }

    /// A page an event asked for is on its way: remember it, and say
    /// whether it was already (#1607). A scroll's asks go through
    /// [`Self::row_at`], which remembers its own; this is for the feed's,
    /// so a change and a reload in one turn cost one read of the page.
    pub fn note_pending(&mut self, page: u32) -> bool {
        self.pending.insert(page)
    }

    /// Whether `page` is on its way, from whichever ask.
    pub fn is_pending(&self, page: u32) -> bool {
        self.pending.contains(&page)
    }

    /// Give up on a page whose fetch failed, so it can be asked for again.
    ///
    /// [`want`](Self::want) refuses to ask twice for a page already on its
    /// way, which is what keeps scrolling a huge folder cheap. That rule
    /// assumes every request is eventually answered one way or the other:
    /// without this, a fetch that *errors* answers nothing, the page stays
    /// pending for the life of the window, and its rows are placeholders
    /// that no scroll, repaint or refresh can clear. A live inbox drew
    /// almost entirely as skeletons that way.
    ///
    /// It does not retry by itself. The page simply becomes askable again,
    /// and the next thing that needs a row in it asks -- which is the same
    /// path a first request takes, and keeps a permanently failing store
    /// from spinning a retry loop of its own.
    ///
    /// Generation-guarded exactly as [`deliver`](Self::deliver) is: a failure
    /// from the folder we have already left says nothing about this one.
    pub fn abandon(&mut self, generation: u64, page: u32) {
        if generation != self.generation {
            return;
        }
        // The failure of a request a splice already disowned says nothing
        // about the one asked since.
        if let Some(count) = self.discard.get_mut(&page)
            && *count > 0
        {
            *count -= 1;
            return;
        }
        self.pending.remove(&page);
    }

    /// Accept a page of rows delivered for `generation`.
    ///
    /// Rows already resident for the same message are reconciled through
    /// [`ListRow::reconcile`] rather than replaced outright, so a
    /// redelivered page does not invalidate anything holding onto them.
    pub fn deliver(&mut self, generation: u64, page: u32, rows: Vec<T>) -> Delivered {
        let disowned = self.discard.get(&page).is_some_and(|count| *count > 0);
        if generation != self.generation || disowned {
            if disowned && generation == self.generation {
                *self.discard.get_mut(&page).expect("just read") -= 1;
            }
            return Delivered {
                stale: true,
                changed: None,
                evicted: Vec::new(),
                reconciled: false,
            };
        }
        self.pending.remove(&page);

        let previous = self.pages.remove(&page);
        // Taken before `existing` consumes them: whether the page came back
        // as the same messages in the same order is a question about
        // position, which a map keyed by id cannot answer.
        let previous_ids: Vec<Option<MessageId>> =
            previous.iter().flatten().map(|row: &T| row.id()).collect();
        let existing: HashMap<MessageId, T> = previous
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

        let reconciled = previous_ids.len() == self.pages[&page].len()
            && previous_ids
                .iter()
                .zip(&self.pages[&page])
                .all(|(before, now)| before.is_some() && *before == now.id());

        Delivered {
            stale: false,
            changed,
            evicted,
            reconciled,
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
    fn resident_rows_are_read_without_asking_for_anything() {
        // A conversation open looked for the rows it already had by walking
        // every position, and every position not held is a page request --
        // five `j` presses in a large folder asked for 138 pages. What is
        // held has to be readable without asking.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(10_000);
        deliver_fresh(&mut window, 0, 10_000);
        deliver_fresh(&mut window, 3, 10_000);

        let mut held: Vec<MessageId> = window.resident().map(|row| row.id).collect();
        held.sort();
        let mut expected: Vec<MessageId> = page_rows(0, 10_000)
            .into_iter()
            .chain(page_rows(3, 10_000))
            .map(|row| row.id)
            .collect();
        expected.sort();
        assert_eq!(held, expected);
        for page in [1, 2, 4] {
            assert!(!window.is_pending(page), "page {page} was asked for");
        }
    }

    // ── refreshing a stretch in place ────────────────────────────────────

    fn ids(values: &[i64]) -> Vec<MessageId> {
        values.iter().map(|value| MessageId::new(*value)).collect()
    }

    /// Run `splices` over `old` the way a toolkit applies it -- each step to
    /// the state the one before left -- and hand back what that produces.
    fn applied(old: &[i64], new: &[i64]) -> (Vec<i64>, Vec<Splice>) {
        let before: Vec<Option<MessageId>> = old.iter().map(|v| Some(MessageId::new(*v))).collect();
        let script = splices(&before, &ids(new));
        let mut current: Vec<i64> = old.to_vec();
        for step in &script {
            match *step {
                Splice::Remove { at, count } => {
                    current.drain(at as usize..(at + count) as usize);
                }
                Splice::Insert { at, count, from } => {
                    for offset in 0..count as usize {
                        current.insert(at as usize + offset, new[from + offset]);
                    }
                }
            }
        }
        (current, script)
    }

    #[test]
    fn an_unchanged_stretch_needs_no_splice_at_all() {
        let (after, script) = applied(&[1, 2, 3, 4], &[1, 2, 3, 4]);
        assert_eq!(after, vec![1, 2, 3, 4]);
        assert!(script.is_empty(), "{script:?}");
    }

    #[test]
    fn mail_at_the_top_is_one_insert_and_what_fell_off_the_end_one_remove() {
        // A page is fifty rows: one new row at the top pushes the last one
        // into the next page. Two positional steps, and every row between
        // keeps its place in the list's eyes -- no replace of the stretch.
        let (after, script) = applied(&[1, 2, 3, 4], &[9, 1, 2, 3]);
        assert_eq!(after, vec![9, 1, 2, 3]);
        assert_eq!(
            script,
            vec![
                Splice::Remove { at: 3, count: 1 },
                Splice::Insert {
                    at: 0,
                    count: 1,
                    from: 0
                },
            ]
        );
    }

    #[test]
    fn a_row_that_left_is_one_remove_and_the_next_page_fills_in_behind() {
        let (after, script) = applied(&[1, 2, 3, 4], &[1, 3, 4, 5]);
        assert_eq!(after, vec![1, 3, 4, 5]);
        assert_eq!(
            script,
            vec![
                Splice::Remove { at: 1, count: 1 },
                Splice::Insert {
                    at: 3,
                    count: 1,
                    from: 3
                },
            ]
        );
    }

    #[test]
    fn a_conversation_that_moved_to_the_top_is_taken_out_and_put_back() {
        let (after, script) = applied(&[1, 2, 3, 4, 5], &[4, 1, 2, 3, 5]);
        assert_eq!(after, vec![4, 1, 2, 3, 5]);
        assert_eq!(script.len(), 2, "{script:?}");
    }

    #[test]
    fn neighbouring_changes_are_announced_as_one_run() {
        let (after, script) = applied(&[1, 2, 3, 4, 5, 6], &[7, 8, 1, 2, 5, 6]);
        assert_eq!(after, vec![7, 8, 1, 2, 5, 6]);
        assert_eq!(
            script,
            vec![
                Splice::Remove { at: 2, count: 2 },
                Splice::Insert {
                    at: 0,
                    count: 2,
                    from: 0
                },
            ]
        );
    }

    #[test]
    fn a_stretch_that_grew_or_shrank_at_the_end_splices_the_end() {
        assert_eq!(applied(&[1, 2], &[1, 2, 3]).0, vec![1, 2, 3]);
        assert_eq!(applied(&[1, 2, 3], &[1, 2]).0, vec![1, 2]);
        assert_eq!(applied(&[1, 2, 3], &[]).0, Vec::<i64>::new());
        assert_eq!(applied(&[], &[4, 5]).0, vec![4, 5]);
    }

    #[test]
    fn a_position_nothing_was_known_about_is_replaced() {
        let before = vec![Some(MessageId::new(1)), None, Some(MessageId::new(3))];
        let script = splices(&before, &ids(&[1, 2, 3]));
        assert_eq!(
            script,
            vec![
                Splice::Remove { at: 1, count: 1 },
                Splice::Insert {
                    at: 1,
                    count: 1,
                    from: 1
                },
            ]
        );
    }

    #[test]
    fn any_old_stretch_splices_into_any_new_one() {
        // Not a proof, but a thousand shapes of it: whatever moved, left or
        // arrived, the steps land exactly on what was read.
        let mut seed: u64 = 0x5eed;
        let mut next = move |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };
        for _ in 0..1000 {
            let old: Vec<i64> = (1..=next(12) as i64).collect();
            let mut new: Vec<i64> = old.iter().copied().filter(|_| next(4) != 0).collect();
            for _ in 0..next(4) {
                let at = next(new.len() as u64 + 1) as usize;
                new.insert(at, 100 + next(50) as i64);
            }
            new.dedup();
            let mut seen = std::collections::HashSet::new();
            new.retain(|id| seen.insert(*id));
            if new.len() > 1 && next(3) == 0 {
                let a = next(new.len() as u64) as usize;
                let moved = new.remove(a);
                new.insert(0, moved);
            }
            assert_eq!(applied(&old, &new).0, new, "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn splicing_a_held_run_reflows_its_pages_and_keeps_the_total_honest() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(200);
        for page in 0..2 {
            deliver_fresh(&mut window, page, 200);
        }
        deliver_fresh(&mut window, 3, 200);
        // One in at the top of the run: every held row moves down one, the
        // last of page 0 becoming the first of page 1.
        assert!(window.splice(0, 0, vec![row(999)]));
        assert_eq!(window.total(), 201);
        assert_eq!(window.peek(0), Some(MessageId::new(1000)));
        assert_eq!(window.peek(50), Some(MessageId::new(50)), "carried across");
        assert_eq!(held(&window, 0).len(), 50);
        assert_eq!(held(&window, 1).len(), 51 - 1, "the run keeps whole pages");
        assert_eq!(
            window.peek(100),
            Some(MessageId::new(100)),
            "the row pushed past the run's last page is held on a new short page"
        );
        assert!(
            !window.resident_pages().contains(&3),
            "a page past the run is one row out now, so it goes"
        );

        // And one out, which pulls the rows after it back up.
        assert!(window.splice(10, 1, Vec::new()));
        assert_eq!(window.total(), 200);
        assert_eq!(window.peek(10), Some(MessageId::new(11)));
        assert_eq!(window.peek(49), Some(MessageId::new(50)));
    }

    #[test]
    fn a_splice_outside_every_held_run_is_refused() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(200);
        deliver_fresh(&mut window, 0, 200);
        assert!(!window.splice(120, 1, Vec::new()));
        assert_eq!(window.total(), 200);
        // The end of a held run is still inside it: that is where a stretch
        // that grew puts its new rows.
        assert!(window.splice(50, 0, vec![row(500)]));
        assert_eq!(window.peek(50), Some(MessageId::new(501)));
    }

    #[test]
    fn a_page_asked_for_before_a_splice_moved_it_is_not_applied_out_of_line() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(200);
        deliver_fresh(&mut window, 0, 200);
        let Some(Lookup::Missing { request }) = window.row_at(120) else {
            panic!("page 2 should have been missing");
        };
        assert!(request.contains(&2));
        assert!(window.splice(0, 0, vec![row(999)]));
        let landed = deliver_fresh(&mut window, 2, 200);
        assert!(
            landed.stale,
            "page 2 was read at the old offsets, one row out from where it would land"
        );
        assert!(!window.is_pending(2), "and it can be asked for again");
    }

    fn held(window: &ListWindow<Fixture>, page: u32) -> Vec<i64> {
        window
            .pages
            .get(&page)
            .map(|rows| rows.iter().map(|row| row.id.get()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn removing_a_row_shifts_the_rows_after_it_up_one() {
        // #1607: an archive reloaded the whole list -- every widget rebuilt,
        // every seek mark dropped -- to take out rows it was holding.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        for page in 0..3 {
            deliver_fresh(&mut window, page, 120);
        }
        assert!(window.remove_at(10));
        assert_eq!(window.total(), 119);
        assert_eq!(
            window.peek(10),
            Some(MessageId::new(12)),
            "the next row moved up"
        );
        assert_eq!(
            window.peek(49),
            Some(MessageId::new(51)),
            "across the page boundary"
        );
        assert_eq!(window.peek(118), Some(MessageId::new(120)), "to the end");
        assert_eq!(held(&window, 2).len(), 19, "the last page is one shorter");
    }

    #[test]
    fn pages_past_a_gap_are_dropped_rather_than_left_one_row_out() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(250);
        for page in [0, 1, 3] {
            deliver_fresh(&mut window, page, 250);
        }
        assert!(window.remove_at(5));
        assert_eq!(window.peek(49), Some(MessageId::new(51)));
        assert_eq!(
            held(&window, 1).len(),
            49,
            "the page before the gap is short: its last row is in the page nobody holds"
        );
        assert!(
            held(&window, 3).is_empty(),
            "a page past the gap is misaligned"
        );
        let wanted = match window.row_at(99) {
            Some(Lookup::Missing { request }) => request,
            _ => Vec::new(),
        };
        assert!(
            wanted.contains(&1),
            "the short page is asked for again when its row is"
        );
    }

    #[test]
    fn a_removal_waits_for_a_page_already_on_its_way() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        deliver_fresh(&mut window, 0, 120);
        assert!(window.note_pending(1));
        assert!(
            !window.remove_at(3),
            "page 1 was asked for at the old offsets"
        );
        assert_eq!(window.total(), 120, "and nothing changed");
        assert!(
            !window.remove_at(100),
            "a row not held cannot be removed in place"
        );
    }

    #[test]
    fn a_page_asked_for_by_an_event_stays_pending_until_it_lands() {
        // #1607: a change and a reload in one main-loop turn both asked for
        // the same page, and nothing remembered the first ask. A page an
        // event asks for is pending like one a scroll asks for, and asking
        // again while it is on its way is answered no.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(120);
        assert!(!window.is_pending(1));
        assert!(window.note_pending(1), "the first ask is new");
        assert!(!window.note_pending(1), "the second ask is not");
        assert!(window.is_pending(1));
        deliver_fresh(&mut window, 1, 120);
        assert!(!window.is_pending(1), "delivery settles it");
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
    fn a_short_middle_page_is_re_requested_rather_than_stuck_as_a_placeholder() {
        // The unsafe case #1165 describes: total was 500 when page 2 was
        // requested, but the store had shrunk to 200 rows by the time the
        // fetch actually ran, so the delivery for page 2 (positions
        // 100..150) came back with only 20 rows in it. `set_total` corrects
        // the count to 200 without touching what is cached -- page 2 stays
        // exactly as short as it arrived, and 200 / 50 = 4 pages means it is
        // still a *middle* page, not the legitimately-short last one.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);
        window.deliver(window.generation(), 2, (100..120).map(row).collect());
        window.set_total(200);

        // Position 130 is index 30 within page 2 -- past the 20 rows the
        // short delivery actually holds, so it must still come back Missing
        // and, critically, must still ask for page 2 rather than treating
        // `pages.contains_key(&2)` as good enough forever.
        match window.row_at(130) {
            Some(Lookup::Missing { request }) => {
                assert!(
                    request.contains(&2),
                    "expected the short page 2 to be re-requested, got {request:?}"
                );
            }
            other => panic!("expected row 130 to be Missing with page 2 requested, got {other:?}"),
        }
    }

    #[test]
    fn a_page_whose_fetch_failed_is_asked_for_again() {
        // Seen on a live account: most of a 709-message inbox drew as
        // skeletons and stayed that way while the backfill ran. "One request
        // per page, ever, until it is evicted" is the rule that keeps
        // scrolling cheap, and it assumed every request is eventually
        // answered. A fetch that *fails* answered nothing, so the page stayed
        // pending for the rest of the session and its fifty rows were
        // placeholders no scroll, repaint or refresh could clear.
        //
        // A banner is not a repair: the rows are still wrong, and the only
        // thing that was ever going to fix them is asking again.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);

        let Some(Lookup::Missing { request }) = window.row_at(130) else {
            panic!("a page nobody has delivered must be Missing");
        };
        assert!(request.contains(&2), "expected page 2 to be asked for");

        // Asking again while it is outstanding is correctly refused...
        match window.row_at(130) {
            Some(Lookup::Missing { request }) => assert!(
                !request.contains(&2),
                "page 2 is already on its way; asking twice is the waste this \
                 rule prevents"
            ),
            other => panic!("expected Missing, got {other:?}"),
        }

        // ...until the fetch comes back empty-handed.
        window.abandon(window.generation(), 2);

        match window.row_at(130) {
            Some(Lookup::Missing { request }) => assert!(
                request.contains(&2),
                "a failed page was never asked for again, so its rows are \
                 skeletons for the rest of the session: {request:?}"
            ),
            other => panic!("expected Missing with page 2 re-requested, got {other:?}"),
        }
    }

    #[test]
    fn an_abandonment_from_a_stale_generation_is_ignored() {
        // The same guard `deliver` has. A fetch that failed against the
        // folder we just left must not un-pend a page of the folder we are
        // in now -- the numbering is per generation, so page 2 there is not
        // page 2 here.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);
        let stale = window.generation();
        let Some(Lookup::Missing { .. }) = window.row_at(130) else {
            panic!("expected page 2 to be asked for");
        };
        window.reset(500);

        window.abandon(stale, 2);

        // Nothing to assert about page 2 directly -- `reset` cleared it. What
        // must hold is that the stale call did not disturb the new window.
        match window.row_at(130) {
            Some(Lookup::Missing { request }) => assert!(request.contains(&2)),
            other => panic!("expected Missing, got {other:?}"),
        }
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
