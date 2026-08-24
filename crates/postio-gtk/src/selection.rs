//! What an action will hit — which is not where the keyboard is.
//!
//! # Two states, not one
//!
//! A list has a **cursor** and a **selection**, and they are different things.
//! The cursor is where the keyboard is: `j` and `k` move it, the reading pane
//! follows it, and canvas 1b draws it with an accent tint, a 3px steel left
//! edge and the key hints. The selection is what `a` would archive. Most of
//! the time they are the same row, which is exactly why conflating them is the
//! usual bug: it only shows up once a selection is more than one row, and then
//! every bulk action lands somewhere the user did not expect.
//!
//! `GtkSingleSelection` is the cursor here, not the selection — the name is
//! GTK's, the meaning is ours. This module is the other half.
//!
//! # Never a `Vec` for "select all"
//!
//! The list is windowed over paged SQLite and must never materialise a mailbox
//! (`docs/PRODUCT.md` §18), so "select all" cannot mean "collect a hundred thousand
//! ids". [`Selection`] — `postio-core`'s, so the view and the command bus
//! cannot drift about what a selection *is* — models it as either an explicit
//! set or the predicate `Everything { except }`. Selecting a 100k mailbox and
//! taking three rows back out is four ids, and archiving it is one statement
//! for the store to resolve rather than 100k of anything.
//!
//! That is why [`summary`] takes the total as an argument instead of counting:
//! counting is the thing the predicate exists to avoid.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use postio_core::state::Selection;
use postio_model::MessageId;

/// A change to the selection, handed to whoever is drawing it.
type Observer = Box<dyn Fn(&Selection)>;

/// The selection, the anchor a range extends from, and who to tell.
///
/// Cheap to clone: every clone is the same selection, which is what lets the
/// rows, the header and the command handlers all hold one.
#[derive(Clone, Default)]
pub struct SelectionState {
    inner: Rc<Inner>,
}

#[derive(Default)]
struct Inner {
    selection: RefCell<Selection>,
    /// Where a range extension counts from — the last row the user pointed
    /// at deliberately, rather than wherever the cursor has since wandered.
    anchor: Cell<Option<MessageId>>,
    observers: RefCell<Vec<Observer>>,
}

impl std::fmt::Debug for SelectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionState")
            .field("selection", &self.selection())
            .field("anchor", &self.inner.anchor.get())
            .finish_non_exhaustive()
    }
}

impl SelectionState {
    /// Nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// What is selected right now.
    pub fn selection(&self) -> Selection {
        self.inner.selection.borrow().clone()
    }

    /// Whether an action would hit nothing.
    pub fn is_empty(&self) -> bool {
        self.inner.selection.borrow().is_empty()
    }

    /// Whether `message` is in the selection.
    pub fn contains(&self, message: MessageId) -> bool {
        self.inner.selection.borrow().contains(message)
    }

    /// The row a range extends from.
    pub fn anchor(&self) -> Option<MessageId> {
        self.inner.anchor.get()
    }

    /// Called whenever the selection changes.
    pub fn connect_changed(&self, observer: impl Fn(&Selection) + 'static) {
        self.inner.observers.borrow_mut().push(Box::new(observer));
    }

    /// Select exactly `message` — a plain click, or opening one.
    pub fn select_only(&self, message: MessageId) {
        self.inner.anchor.set(Some(message));
        self.replace(Selection::These(vec![message]));
    }

    /// Add `message` if it is out, take it out if it is in — `x`, and
    /// Ctrl-click.
    pub fn toggle(&self, message: MessageId) {
        self.inner.anchor.set(Some(message));
        self.mutate(|selection| selection.toggle(message));
    }

    /// Add `message`, leaving it alone if it is already in — `J` and `K`.
    ///
    /// The anchor does not move: extending is one gesture however many times
    /// it is repeated, so a later Shift-click still counts from where the
    /// user started rather than from the last row it reached.
    pub fn extend_to(&self, message: MessageId) {
        if self.inner.anchor.get().is_none() {
            self.inner.anchor.set(Some(message));
        }
        self.mutate(|selection| selection.insert(message));
    }

    /// Add every message in `messages` — a Shift-click over a range.
    pub fn extend_over(&self, messages: impl IntoIterator<Item = MessageId>) {
        self.mutate(|selection| {
            for message in messages {
                selection.insert(message);
            }
        });
    }

    /// Select everything the list is showing, without naming any of it.
    pub fn select_all(&self) {
        self.replace(Selection::Everything { except: Vec::new() });
    }

    /// Drop the selection, and the anchor with it.
    pub fn clear(&self) {
        self.inner.anchor.set(None);
        self.replace(Selection::default());
    }

    /// Replace the selection wholesale — what an event from the command bus
    /// will do once `postio-agr` makes the bus the writer.
    pub fn set(&self, selection: Selection) {
        self.replace(selection);
    }

    fn replace(&self, selection: Selection) {
        if *self.inner.selection.borrow() == selection {
            return;
        }
        self.inner.selection.replace(selection);
        self.announce();
    }

    fn mutate(&self, change: impl FnOnce(&mut Selection)) {
        let before = self.inner.selection.borrow().clone();
        change(&mut self.inner.selection.borrow_mut());
        if *self.inner.selection.borrow() != before {
            self.announce();
        }
    }

    fn announce(&self) {
        let selection = self.inner.selection.borrow().clone();
        for observer in self.inner.observers.borrow().iter() {
            observer(&selection);
        }
    }
}

/// What the header says about the selection, or `None` when there is nothing
/// to say.
///
/// `total` is how many rows the list holds, which the store already knows from
/// the count it does to size the window. Without it a predicate can only be
/// described rather than counted — "everything" is a promise about a query,
/// and answering "how many" by walking it is the one thing the predicate
/// exists to prevent.
pub fn summary(selection: &Selection, total: Option<u32>) -> Option<String> {
    match selection {
        Selection::These(messages) if messages.is_empty() => None,
        Selection::These(messages) => Some(format!("{} selected", count(messages.len() as u32))),
        Selection::Everything { except } => match total {
            Some(total) => {
                let selected = total.saturating_sub(except.len() as u32);
                Some(format!("{} selected", count(selected)))
            }
            None => Some("All selected".to_owned()),
        },
    }
}

/// A count with thousands separated, because a selection is a number the user
/// is about to act on and "50000" is not a number anybody reads.
fn count(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The messages between two positions in the list, inclusive, in list order.
///
/// `rows` is the list as it stands, `None` where a page has not arrived yet.
/// Those gaps are skipped rather than waited for: the user is dragging a
/// range they can see, and a selection that blocks on a page fetch would be a
/// selection that stutters.
pub fn range(rows: &[Option<MessageId>], from: usize, to: usize) -> Vec<MessageId> {
    let (first, last) = if from <= to { (from, to) } else { (to, from) };
    rows.iter()
        .skip(first)
        .take(last.saturating_sub(first) + 1)
        .flatten()
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: i64) -> MessageId {
        MessageId::new(value)
    }

    #[test]
    fn a_plain_click_replaces_whatever_was_selected() {
        let state = SelectionState::new();
        state.toggle(id(1));
        state.toggle(id(2));

        state.select_only(id(7));

        assert!(state.contains(id(7)));
        assert!(!state.contains(id(1)));
        assert_eq!(state.anchor(), Some(id(7)));
    }

    #[test]
    fn toggling_moves_the_anchor_and_extending_does_not() {
        // Ctrl-click says "count from here"; Shift+J says "carry on from
        // where I started". A range extended one row at a time has to end up
        // where the same range dragged in one gesture would.
        let state = SelectionState::new();
        state.toggle(id(1));
        state.extend_to(id(2));
        state.extend_to(id(3));

        assert_eq!(state.anchor(), Some(id(1)));
        assert!([1, 2, 3].iter().all(|n| state.contains(id(*n))));
    }

    #[test]
    fn select_all_is_a_predicate_rather_than_a_hundred_thousand_ids() {
        let state = SelectionState::new();
        state.select_all();

        assert!(state.selection().is_everything());
        assert_eq!(state.selection().ids(), None, "nothing was materialised");
        assert!(state.contains(id(98_765)), "including rows never loaded");
    }

    #[test]
    fn taking_a_row_out_of_everything_edits_the_exceptions() {
        let state = SelectionState::new();
        state.select_all();

        state.toggle(id(4));

        assert!(state.selection().is_everything(), "still the predicate");
        assert!(!state.contains(id(4)));
        assert!(state.contains(id(5)));
    }

    #[test]
    fn observers_hear_a_change_and_not_a_repetition() {
        let state = SelectionState::new();
        let heard = Rc::new(Cell::new(0));
        let counter = heard.clone();
        state.connect_changed(move |_| counter.set(counter.get() + 1));

        state.select_only(id(1));
        state.select_only(id(1));
        state.clear();
        state.clear();

        assert_eq!(heard.get(), 2, "one for the selection, one for the clear");
    }

    #[test]
    fn an_empty_selection_says_nothing() {
        assert_eq!(summary(&Selection::default(), Some(40)), None);
    }

    #[test]
    fn a_named_selection_counts_itself() {
        let selection = Selection::These(vec![id(1), id(2), id(3)]);

        assert_eq!(summary(&selection, None).as_deref(), Some("3 selected"));
    }

    #[test]
    fn a_predicate_counts_from_the_total_the_store_already_knows() {
        let selection = Selection::Everything {
            except: vec![id(1), id(2)],
        };

        assert_eq!(
            summary(&selection, Some(50_000)).as_deref(),
            Some("49,998 selected"),
            "counted by arithmetic, never by walking the mailbox"
        );
    }

    #[test]
    fn a_predicate_with_no_total_is_described_rather_than_counted() {
        let selection = Selection::Everything { except: Vec::new() };

        assert_eq!(summary(&selection, None).as_deref(), Some("All selected"));
    }

    #[test]
    fn a_range_runs_in_list_order_whichever_end_it_was_dragged_from() {
        let rows: Vec<Option<MessageId>> = (1..=5).map(|n| Some(id(n))).collect();

        assert_eq!(range(&rows, 1, 3), vec![id(2), id(3), id(4)]);
        assert_eq!(
            range(&rows, 3, 1),
            vec![id(2), id(3), id(4)],
            "dragging upwards selects the same rows"
        );
    }

    #[test]
    fn a_range_skips_rows_whose_page_has_not_arrived() {
        let rows = vec![Some(id(1)), None, Some(id(3))];

        assert_eq!(range(&rows, 0, 2), vec![id(1), id(3)]);
    }

    #[test]
    fn a_range_of_one_row_is_that_row() {
        let rows = vec![Some(id(1)), Some(id(2))];

        assert_eq!(range(&rows, 1, 1), vec![id(2)]);
    }
}
