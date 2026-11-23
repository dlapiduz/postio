//! The message list's paging and generation bookkeeping, shared by every frontend (ADR 0019 Q5a).
//! `ListWindow` owns the resident rows, so the memory bound is structural; it must never be
//! called from inside `GListModel::item()`. No method is fallible, blocking or async.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use postio_core::aim::{RowFacts, RowKind};
use postio_model::ids::{MessageId, ThreadId};

pub const PAGE_SIZE: u32 = 50;

pub const CACHE_PAGES: usize = 8;

pub trait ListRow {
    /// `None` for a row not yet loaded (a placeholder).
    fn id(&self) -> Option<MessageId>;

    /// `None` means a message row. The one question `postio_core::aim` asks a frontend's list.
    fn thread(&self) -> Option<ThreadId> {
        None
    }

    /// A thread's row is drawn from its newest message, so a reply changes `id` while the row is
    /// the same one; compared by message, the cursor slid to the row below (#2107).
    fn key(&self) -> Option<RowKey> {
        self.thread()
            .map(RowKey::Thread)
            .or_else(|| self.id().map(RowKey::Message))
    }

    /// The default takes the incoming value; GTK overrides it to update the existing `GObject` in
    /// place.
    fn reconcile(_existing: &Self, incoming: Self) -> Self
    where
        Self: Sized,
    {
        incoming
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKey {
    Thread(ThreadId),
    Message(MessageId),
    Other(u8, i64),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<'a, T> {
    Resident(&'a T),
    Missing { request: Vec<u32> },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Delivered {
    /// Dropped: a reply from a generation the user has left. Nothing else means anything when this
    /// is true.
    pub stale: bool,
    pub changed: Option<Range<u32>>,
    pub evicted: Vec<u32>,
    /// Every row landed on a resident row, in the same order and number. A toolkit list answers
    /// a replacement by rebuilding the widgets in range, so announcing a refetched page as one
    /// blinks the list for one flag change.
    pub reconciled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Splice {
    Remove { at: u32, count: u32 },
    Insert { at: u32, count: u32, from: usize },
}

/// Removals first, bottom up, then insertions, top down, each counted in the state the earlier
/// steps left. A row whose key is in both, in the same relative order, stays and is updated in
/// place, so a resync does not rebuild every widget. A position `old` knew nothing about is always
/// replaced.
pub fn splices<K: Copy + Eq + std::hash::Hash>(old: &[Option<K>], new: &[K]) -> Vec<Splice> {
    let mut held: HashMap<K, usize> = HashMap::with_capacity(old.len());
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

pub struct ListWindow<T> {
    total: u32,
    pages: HashMap<u32, Vec<T>>,
    recent: VecDeque<u32>,
    pending: HashSet<u32>,
    generation: u64,
    /// Replies to drop when they land, per page: asked for at offsets a splice has since moved.
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn resident_rows(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    pub fn resident_pages(&self) -> Vec<u32> {
        let mut pages: Vec<u32> = self.pages.keys().copied().collect();
        pages.sort_unstable();
        pages
    }

    /// Empties the cache and bumps the generation, so a reply in flight for the old scope is
    /// dropped. Returns the new generation.
    pub fn reset(&mut self, total: u32) -> u64 {
        let next = self.generation + 1;
        self.adopt(total, next);
        next
    }

    /// `reset` to a generation the caller already handed out, for a list that keeps showing the old
    /// scope until the new one's first page lands.
    pub fn adopt(&mut self, total: u32, generation: u64) {
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.discard.clear();
        self.total = total;
        self.generation = generation;
    }

    /// Keeps every held page but drops replies asked for under the old generation.
    pub fn renumber(&mut self, generation: u64) {
        self.pending.clear();
        self.discard.clear();
        self.generation = generation;
    }

    /// For when the order itself moved (a resync, a re-sort): the generation is kept, so requests
    /// in flight are not stale. Returns the row count.
    pub fn invalidate(&mut self) -> u32 {
        self.pages.clear();
        self.recent.clear();
        self.pending.clear();
        self.discard.clear();
        self.total
    }

    /// Returns `(position, removed, added)` in `items_changed` shape, or `None` if the total did
    /// not change.
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

    /// Every cached page is misaligned, so the cache is dropped; a refetch is cheaper. The
    /// generation is not bumped. Returns whether anything happened.
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

    /// `false`, with nothing changed, when the row is not resident or a later page is still on its
    /// way (it would land one row out); the caller reloads.
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

        // Each following held page gives its first row to the one before; the last is then one row
        // short, which `row_at` asks for again (#1165).
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
        self.pages.retain(|held, _| *held <= last);
        self.recent.retain(|held| *held <= last);
        true
    }

    /// Replaces `removed` rows at `position` inside the run of held pages that holds it, re-cutting
    /// the run into pages.
    /// If its length changes, pages held past it are dropped and a page asked for at the old
    /// offsets has its answer dropped on landing. `false` when no held run covers the change.
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

    fn rechunk(&mut self, first: u32, rows: Vec<T>) {
        let mut page = first;
        let mut rest = rows.into_iter().peekable();
        while rest.peek().is_some() {
            let chunk: Vec<T> = rest.by_ref().take(PAGE_SIZE as usize).collect();
            self.pages.insert(page, chunk);
            page += 1;
        }
    }

    /// For a refresh that re-read only the visible pages: the rest are one row out per row moved,
    /// so they go.
    pub fn retain_pages(&mut self, keep: impl Fn(u32) -> bool) {
        self.pages.retain(|page, _| keep(*page));
        self.recent.retain(|page| keep(*page));
    }

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

    pub fn resident(&self) -> impl Iterator<Item = &T> {
        self.pages.values().flatten()
    }

    /// `None` for a position outside the total.
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

        // Ask for the page and its neighbours so scrolling does not stall at a boundary. The page
        // being read must cover `index`, not merely be present (#1165); neighbours are prefetch.
        let mut request = Vec::with_capacity(3);
        self.want(page, Some(index), &mut request);
        if page > 0 {
            self.want(page - 1, None, &mut request);
        }
        self.want(page + 1, None, &mut request);
        Some(Lookup::Missing { request })
    }

    /// A page delivered short (the last, or one whose fetch raced a shrinking store) must not
    /// read as resident forever: with `required_index` that row must be present, else the position
    /// sticks on a placeholder.
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

    /// For the feed's asks, so a change and a reload in one turn cost one read of the page (#1607).
    pub fn note_pending(&mut self, page: u32) -> bool {
        self.pending.insert(page)
    }

    pub fn is_pending(&self, page: u32) -> bool {
        self.pending.contains(&page)
    }

    /// `want` refuses to ask twice for a pending page, which assumes every request is answered;
    /// without this a failed fetch leaves its rows placeholders forever. It does not retry itself.
    /// Generation-guarded like `deliver`.
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
        // Taken before `existing` consumes them: sameness of order is a question about position.
        let previous_keys: Vec<Option<RowKey>> =
            previous.iter().flatten().map(|row: &T| row.key()).collect();
        let existing: HashMap<RowKey, T> = previous
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row.key()?, row)))
            .collect();

        let count = rows.len() as u32;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let reconciled = match row.key().and_then(|key| existing.get(&key)) {
                Some(old) => T::reconcile(old, row),
                None => row,
            };
            items.push(reconciled);
        }

        self.pages.insert(page, items);
        self.touch(page);
        let evicted = self.evict();

        let start = page * PAGE_SIZE;
        let span = count.min(self.total.saturating_sub(start));
        let changed = (span > 0).then_some(start..start + span);

        let reconciled = previous_keys.len() == self.pages[&page].len()
            && previous_keys
                .iter()
                .zip(&self.pages[&page])
                .all(|(before, now)| before.is_some() && *before == now.key());

        Delivered {
            stale: false,
            changed,
            evicted,
            reconciled,
        }
    }

    /// Reconciled like a redelivered page. Returns the position if resident; a row not on screen is
    /// read fresh when next shown.
    pub fn update(&mut self, incoming: T) -> Option<u32> {
        let key = incoming.key()?;
        let found = self.pages.iter().find_map(|(page, rows)| {
            let index = rows.iter().position(|row| row.key() == Some(key))?;
            Some((*page, index))
        })?;
        let (page, index) = found;
        let rows = self.pages.get_mut(&page).expect("just found it");
        let updated = T::reconcile(&rows[index], incoming);
        rows[index] = updated;
        Some(page * PAGE_SIZE + index as u32)
    }

    /// Resident only: a shift-click across ten thousand rows must not ask the store for ten
    /// thousand rows.
    pub fn peek(&self, position: u32) -> Option<MessageId> {
        if position >= self.total {
            return None;
        }
        self.pages
            .get(&(position / PAGE_SIZE))
            .and_then(|rows| rows.get((position % PAGE_SIZE) as usize))
            .and_then(T::id)
    }

    pub fn peek_key(&self, position: u32) -> Option<RowKey> {
        self.resident_at(position).and_then(T::key)
    }

    pub fn resident_at(&self, position: u32) -> Option<&T> {
        if position >= self.total {
            return None;
        }
        self.pages
            .get(&(position / PAGE_SIZE))
            .and_then(|rows| rows.get((position % PAGE_SIZE) as usize))
    }

    /// `None` covers both "not in this scope" and "not fetched yet"; the caller asks for the page
    /// and retries.
    pub fn position_of(&self, message: MessageId) -> Option<u32> {
        self.pages.iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.id() == Some(message))
                .map(|index| page * PAGE_SIZE + index as u32)
        })
    }

    pub fn position_of_key(&self, key: RowKey) -> Option<u32> {
        self.pages.iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.key() == Some(key))
                .map(|index| page * PAGE_SIZE + index as u32)
        })
    }

    /// Resident-only: a fetch would make a keystroke wait on the store.
    pub fn row_of(&self, message: MessageId) -> Option<&T> {
        self.pages
            .values()
            .find_map(|rows| rows.iter().find(|row| row.id() == Some(message)))
    }

    pub fn page_of(&self, message: MessageId) -> Option<u32> {
        self.position_of(message)
            .map(|position| position / PAGE_SIZE)
    }

    pub fn pages_holding(&self, messages: &[MessageId]) -> Vec<u32> {
        let mut pages: Vec<u32> = messages
            .iter()
            .filter_map(|message| self.page_of(*message))
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }

    fn touch(&mut self, page: u32) {
        self.recent.retain(|p| *p != page);
        self.recent.push_back(page);
    }

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

    fn deliver_fresh(window: &mut ListWindow<Fixture>, page: u32, total: u32) -> Delivered {
        window.deliver(window.generation(), page, page_rows(page, total))
    }

    #[test]
    fn resident_rows_are_read_without_asking_for_anything() {
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

    #[test]
    fn a_resident_row_is_read_by_position_and_a_missing_one_asks_for_nothing() {
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(10_000);
        deliver_fresh(&mut window, 2, 10_000);
        assert_eq!(
            window.resident_at(PAGE_SIZE * 2 + 7).map(|row| row.id),
            Some(row(PAGE_SIZE * 2 + 7).id)
        );
        assert_eq!(
            window.resident_at(PAGE_SIZE * 3),
            None,
            "page 3 is not here"
        );
        assert_eq!(window.resident_at(20_000), None, "past the end");
        assert!(!window.is_pending(3), "and nothing was asked for");
    }

    fn ids(values: &[i64]) -> Vec<MessageId> {
        values.iter().map(|value| MessageId::new(*value)).collect()
    }

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

        deliver_fresh(&mut window, 1800, HUGE);
        deliver_fresh(&mut window, 1799, HUGE);
        deliver_fresh(&mut window, 1801, HUGE);
        assert!(matches!(window.row_at(90_000), Some(Lookup::Resident(_))));
    }

    #[test]
    fn a_short_middle_page_is_re_requested_rather_than_stuck_as_a_placeholder() {
        // Page 2 arrives short after the store shrank (#1165); it is a middle page, not the
        // legitimately short last one.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);
        window.deliver(window.generation(), 2, (100..120).map(row).collect());
        window.set_total(200);

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
        // A failed fetch left its page pending forever, so its rows stayed placeholders no scroll
        // or refresh cleared; asking again is the only repair.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);

        let Some(Lookup::Missing { request }) = window.row_at(130) else {
            panic!("a page nobody has delivered must be Missing");
        };
        assert!(request.contains(&2), "expected page 2 to be asked for");

        match window.row_at(130) {
            Some(Lookup::Missing { request }) => assert!(
                !request.contains(&2),
                "page 2 is already on its way; asking twice is the waste this \
                 rule prevents"
            ),
            other => panic!("expected Missing, got {other:?}"),
        }

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
        // A fetch that failed against the folder just left must not un-pend a page of the current
        // one.
        let mut window: ListWindow<Fixture> = ListWindow::new();
        window.reset(500);
        let stale = window.generation();
        let Some(Lookup::Missing { .. }) = window.row_at(130) else {
            panic!("expected page 2 to be asked for");
        };
        window.reset(500);

        window.abandon(stale, 2);

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

/// Blanket, so neither frontend writes one. The `Missing` arm is what a marked row that has been
/// paged out reports, so the shared rules decline to guess (#468).
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

    #[test]
    fn a_row_the_window_does_not_hold_is_missing_rather_than_a_message() {
        let window = window_holding(vec![Row {
            id: MessageId::new(7),
            thread: Some(ThreadId::new(3)),
        }]);

        assert_eq!(window.row_kind(MessageId::new(99)), RowKind::Missing);
    }

    #[test]
    fn a_threads_row_stays_the_same_row_when_a_reply_changes_its_message() {
        let thread = |id: i64, thread: i64| Row {
            id: MessageId::new(id),
            thread: Some(ThreadId::new(thread)),
        };
        let mut window = window_holding(vec![thread(7, 3), thread(8, 4)]);
        let held: Vec<Option<RowKey>> = (0..2).map(|at| window.peek_key(at)).collect();
        let reread: Vec<RowKey> = [thread(9, 3), thread(8, 4)]
            .iter()
            .filter_map(ListRow::key)
            .collect();
        assert_eq!(
            splices(&held, &reread),
            Vec::new(),
            "the thread's row is still the first row"
        );
        assert_eq!(window.update(thread(9, 3)), Some(0), "and is told so");
        assert_eq!(window.peek(0), Some(MessageId::new(9)));
    }

    #[test]
    fn an_empty_window_holds_no_rows_rather_than_fetching_any() {
        let window: ListWindow<Row> = ListWindow::new();
        assert_eq!(window.row_kind(MessageId::new(7)), RowKind::Missing);
        assert!(window.row_of(MessageId::new(7)).is_none());
    }
}
