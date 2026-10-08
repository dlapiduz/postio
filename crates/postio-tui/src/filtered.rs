//! The Filtered view's state (terminal.md, "Filtered"): the reason tab on
//! screen, each tab's count, and the rows read so far, fifty at a time.
//!
//! What it reads and how it is drawn are elsewhere (`crate::ask`,
//! `view::filtered`); this holds which row the keyboard is on, which rows
//! are in view, and when the next page is wanted: only once the last row
//! read is in view, so a long Filtered is never read whole.

use chrono::{DateTime, NaiveDate, Utc};
use postio_client::protocol::FilteredRow;
use postio_model::MessageId;
use postio_ui::filtered;
use postio_ui::terminal::SafeText;

use crate::ask::Ask;
use crate::row::Row;

/// One filtered message: its list row, and why it was filed away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The message as a list row; `when` is when it was filed.
    pub row: Row,
    /// The reason pill: "notification · Forge".
    pub pill: SafeText,
}

impl Item {
    /// The row the store gave, made safe to draw.
    fn of(found: FilteredRow) -> Item {
        let pill = SafeText::new(&filtered::pill(&found.reason, found.source.as_deref()));
        let mut row = Row::from(found.message);
        row.when = found.at;
        Item { row, pill }
    }

    /// When it was filed away.
    pub fn at(&self) -> DateTime<Utc> {
        self.row.when
    }
}

/// One line of the body: a day's heading, or a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The heading of the day whose first row is the `usize`th.
    Heading(usize),
    /// The row at this place among those read.
    Item(usize),
}

/// The view. See the module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtered {
    /// The tab on screen, `0` for All.
    tab: usize,
    /// Each tab's count, as last read.
    counts: [u32; 7],
    /// The rows read, newest first.
    items: Vec<Item>,
    /// The row the keyboard is on.
    cursor: usize,
    /// The first line in view.
    top: usize,
    /// Whether the last page read was full, so there may be more.
    more: bool,
    /// The offset of the page being read, when one is.
    asked: Option<u32>,
    /// Moves with every reading from the top, so an older answer is dropped.
    generation: u64,
    /// The message the keyboard stays on when the first page is read again.
    keep: Option<MessageId>,
}

impl Filtered {
    /// The view, on All, with nothing read.
    pub fn new() -> Filtered {
        Filtered::default()
    }

    /// The tab on screen, `0` for All.
    pub fn tab(&self) -> usize {
        self.tab
    }

    /// Each tab's count, in [`filtered::TABS`]' order.
    pub fn counts(&self) -> [u32; 7] {
        self.counts
    }

    /// The rows read.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The row the keyboard is on, by its place among those read.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The row the keyboard is on.
    pub fn focused(&self) -> Option<&Item> {
        self.items.get(self.cursor)
    }

    /// The first line in view.
    pub fn top(&self) -> usize {
        self.top
    }

    /// The lines of the body, each day under its heading.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots = Vec::with_capacity(self.items.len() + 8);
        let mut day: Option<NaiveDate> = None;
        for (index, item) in self.items.iter().enumerate() {
            let today = item.row.day();
            if day != Some(today) {
                day = Some(today);
                slots.push(Slot::Heading(index));
            }
            slots.push(Slot::Item(index));
        }
        slots
    }

    /// The heading of the day whose first row is the `first`th: the day and
    /// how many of its rows are read.
    pub fn heading(&self, first: usize, today: NaiveDate) -> String {
        let days: Vec<NaiveDate> = self.items.iter().map(|item| item.row.day()).collect();
        filtered::day_headings(&days, today)
            .into_iter()
            .nth(first)
            .flatten()
            .unwrap_or_default()
    }

    /// Read from the top again: the counts and the first page. The keyboard
    /// stays on the message it is on when that is still there.
    pub fn reload(&mut self) -> Vec<Ask> {
        self.keep = self.focused().map(|item| item.row.id);
        vec![Ask::FilteredTabs, self.first_page()]
    }

    /// Show the `index`th tab, from its first page.
    pub fn select_tab(&mut self, index: usize) -> Vec<Ask> {
        if filtered::tab_reason(index).is_none() {
            return Vec::new();
        }
        self.tab = index;
        self.keep = None;
        self.cursor = 0;
        self.top = 0;
        vec![self.first_page()]
    }

    fn reason(&self) -> Option<String> {
        filtered::tab_reason(self.tab).flatten().map(str::to_owned)
    }

    fn first_page(&mut self) -> Ask {
        self.generation += 1;
        self.asked = Some(0);
        self.more = false;
        Ask::FilteredPage {
            generation: self.generation,
            reason: self.reason(),
            offset: 0,
        }
    }

    /// The counts arrived.
    pub fn counted(&mut self, reasons: &[(String, u32)]) {
        self.counts = filtered::tab_counts(reasons);
    }

    /// A page arrived. Answers whether it was for the reading on screen.
    pub fn landed(&mut self, generation: u64, offset: u32, rows: Vec<FilteredRow>) -> bool {
        if generation != self.generation || self.asked != Some(offset) {
            return false;
        }
        self.asked = None;
        self.more = filtered::page_is_full(rows.len());
        let rows = rows.into_iter().map(Item::of);
        if offset == 0 {
            let before = self.cursor;
            self.items = rows.collect();
            self.cursor = self
                .keep
                .take()
                .and_then(|kept| self.items.iter().position(|item| item.row.id == kept))
                .unwrap_or_else(|| before.min(self.items.len().saturating_sub(1)));
        } else {
            self.items.extend(rows);
        }
        true
    }

    /// A page could not be read: nothing more is expected of it.
    pub fn failed(&mut self) {
        self.asked = None;
    }

    /// Move the keyboard by `by` rows, kept to the rows read.
    pub fn step(&mut self, by: isize) {
        let last = self.items.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Move the keyboard to the row at `at`, when there is one.
    pub fn go_to(&mut self, at: usize) {
        if at < self.items.len() {
            self.cursor = at;
        }
    }

    /// Move the keyboard to the last row read.
    pub fn last(&mut self) {
        self.cursor = self.items.len().saturating_sub(1);
    }

    /// Scroll by `lines`, never past the last line, and keep the keyboard
    /// on a row in view.
    pub fn scroll(&mut self, lines: isize, height: usize) {
        let slots = self.slots();
        let last = slots.len().saturating_sub(height.min(slots.len()));
        self.top = self.top.saturating_add_signed(lines).min(last);
        let Some(first) = slots[self.top.min(slots.len().saturating_sub(1))..]
            .iter()
            .find_map(|slot| match slot {
                Slot::Item(index) => Some(*index),
                Slot::Heading(_) => None,
            })
        else {
            return;
        };
        let end = slots
            .iter()
            .skip(self.top)
            .take(height)
            .filter_map(|slot| match slot {
                Slot::Item(index) => Some(*index),
                Slot::Heading(_) => None,
            })
            .next_back()
            .unwrap_or(first);
        self.cursor = self.cursor.clamp(first, end);
    }

    /// Keep the row the keyboard is on in view, with its day's heading when
    /// it is the day's first, in a body `height` lines tall.
    pub fn reveal(&mut self, height: usize) {
        let slots = self.slots();
        let Some(at) = slots
            .iter()
            .position(|slot| *slot == Slot::Item(self.cursor))
        else {
            self.top = 0;
            return;
        };
        let needs = if at > 0 && matches!(slots[at - 1], Slot::Heading(_)) {
            at - 1
        } else {
            at
        };
        if needs < self.top {
            self.top = needs;
        } else if at >= self.top + height.max(1) {
            self.top = at + 1 - height.max(1);
        }
    }

    /// The next page, when the last row read is in view, there may be more,
    /// and none is on its way.
    pub fn wants_more(&mut self, height: usize) -> Option<Ask> {
        if !self.more || self.asked.is_some() || self.items.is_empty() {
            return None;
        }
        let slots = self.slots();
        if self.top + height < slots.len() {
            return None;
        }
        let offset = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        self.asked = Some(offset);
        Some(Ask::FilteredPage {
            generation: self.generation,
            reason: self.reason(),
            offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::filtered_row;

    fn read(count: usize) -> Vec<FilteredRow> {
        (0..count)
            .map(|at| filtered_row(at as i64 + 1, "Forge", "notification", 23, 9, 0))
            .collect()
    }

    #[test]
    fn the_next_page_is_asked_for_only_once_the_last_row_read_is_in_view() {
        let mut view = Filtered::new();
        view.reload();
        assert!(view.landed(1, 0, read(50)));
        // Ten lines in view of fifty-one: not yet.
        assert_eq!(view.wants_more(10), None);
        view.scroll(100, 10);
        let ask = view.wants_more(10);
        assert_eq!(
            ask,
            Some(Ask::FilteredPage {
                generation: 1,
                reason: None,
                offset: 50
            })
        );
        assert_eq!(
            view.wants_more(10),
            None,
            "and not again while it is on its way"
        );
    }

    #[test]
    fn a_short_page_is_the_last() {
        let mut view = Filtered::new();
        view.reload();
        view.landed(1, 0, read(3));
        assert_eq!(view.wants_more(10), None);
    }

    #[test]
    fn an_answer_for_an_earlier_reading_is_dropped() {
        let mut view = Filtered::new();
        view.reload();
        view.select_tab(2);
        assert!(!view.landed(1, 0, read(3)), "the first reading is over");
        assert!(view.landed(2, 0, read(2)));
        assert_eq!(view.items().len(), 2);
    }

    #[test]
    fn reading_again_keeps_the_keyboard_on_its_message_or_near_it() {
        let mut view = Filtered::new();
        view.reload();
        view.landed(1, 0, read(5));
        view.step(2);
        let kept = view.focused().map(|item| item.row.id);
        view.reload();
        let mut again = read(5);
        again.remove(0);
        view.landed(2, 0, again);
        assert_eq!(view.focused().map(|item| item.row.id), kept);
        view.reload();
        view.landed(3, 0, read(1));
        assert_eq!(view.cursor(), 0, "and the last row when it is gone");
    }

    #[test]
    fn the_keyboard_stays_on_the_rows_read_and_in_view() {
        let mut view = Filtered::new();
        view.reload();
        view.landed(1, 0, read(30));
        view.step(-3);
        assert_eq!(view.cursor(), 0);
        view.last();
        view.reveal(10);
        assert_eq!(view.cursor(), 29);
        assert!(view.top() > 0 && view.top() + 10 >= 31);
        view.go_to(0);
        view.reveal(10);
        assert_eq!(view.top(), 0, "the first row shows its day's heading too");
    }

    #[test]
    fn each_day_has_one_heading_with_its_rows_read() {
        let mut view = Filtered::new();
        view.reload();
        let mut rows = read(2);
        rows.push(filtered_row(9, "Forge", "notification", 22, 9, 0));
        view.landed(1, 0, rows);
        assert_eq!(
            view.slots(),
            vec![
                Slot::Heading(0),
                Slot::Item(0),
                Slot::Item(1),
                Slot::Heading(2),
                Slot::Item(2)
            ]
        );
    }
}
