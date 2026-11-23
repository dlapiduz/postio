//! What an action will hit, which is not where the keyboard is: the cursor (keyboard, reading pane)
//! and the selection (what `a` would archive) are separate states.

use std::cell::RefCell;
use std::rc::Rc;

use postio_core::state::Selection;
use postio_model::{AccountId, MessageId};

type Observer = Box<dyn Fn(&Selection)>;

/// A whole-view selection in the unified list is about fewer accounts than the rows on screen,
/// so the ids and the omitted account names travel together (ADR 0005 Q10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    pub accounts: Vec<AccountId>,
    pub omitted: Vec<String>,
}

/// `Send`, so a controller not tied to one thread can own one. Every change returns whether
/// anything changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    selection: Selection,
    /// Meaningless unless the selection is `Everything`.
    reach: Reach,
    /// The last row deliberately pointed at, not wherever the cursor has wandered.
    anchor: Option<MessageId>,
}

impl Selector {
    pub fn selection(&self) -> Selection {
        self.selection.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
    }

    pub fn contains(&self, message: MessageId) -> bool {
        self.selection.contains(message)
    }

    pub fn anchor(&self) -> Option<MessageId> {
        self.anchor
    }

    pub fn select_only(&mut self, message: MessageId) -> bool {
        self.anchor = Some(message);
        self.replace(Selection::These(vec![message]))
    }

    pub fn toggle(&mut self, message: MessageId) -> bool {
        self.anchor = Some(message);
        self.mutate(|selection| selection.toggle(message))
    }

    /// The anchor does not move: extending is one gesture however often it repeats.
    pub fn extend_to(&mut self, message: MessageId) -> bool {
        if self.anchor.is_none() {
            self.anchor = Some(message);
        }
        self.mutate(|selection| selection.insert(message))
    }

    pub fn extend_over(&mut self, messages: impl IntoIterator<Item = MessageId>) -> bool {
        self.mutate(|selection| {
            for message in messages {
                selection.insert(message);
            }
        })
    }

    pub fn select_all(&mut self, reach: Reach) -> bool {
        self.reach = reach;
        self.replace(Selection::Everything { except: Vec::new() })
    }

    pub fn reach(&self) -> Reach {
        match &self.selection {
            Selection::Everything { .. } => self.reach.clone(),
            Selection::These(_) => Reach::default(),
        }
    }

    pub fn clear(&mut self) -> bool {
        self.anchor = None;
        self.replace(Selection::default())
    }

    pub fn set(&mut self, selection: Selection) -> bool {
        self.replace(selection)
    }

    fn replace(&mut self, selection: Selection) -> bool {
        if self.selection == selection {
            return false;
        }
        self.selection = selection;
        true
    }

    fn mutate(&mut self, change: impl FnOnce(&mut Selection)) -> bool {
        let before = self.selection.clone();
        change(&mut self.selection);
        self.selection != before
    }
}

/// Cheap to clone: every clone is the same selection.
#[derive(Clone, Default)]
pub struct SelectionState {
    inner: Rc<Inner>,
}

#[derive(Default)]
struct Inner {
    selector: RefCell<Selector>,
    observers: RefCell<Vec<Observer>>,
}

impl std::fmt::Debug for SelectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionState")
            .field("selection", &self.selection())
            .field("anchor", &self.anchor())
            .finish_non_exhaustive()
    }
}

impl SelectionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selection(&self) -> Selection {
        self.inner.selector.borrow().selection()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.selector.borrow().is_empty()
    }

    pub fn contains(&self, message: MessageId) -> bool {
        self.inner.selector.borrow().contains(message)
    }

    pub fn anchor(&self) -> Option<MessageId> {
        self.inner.selector.borrow().anchor()
    }

    pub fn connect_changed(&self, observer: impl Fn(&Selection) + 'static) {
        self.inner.observers.borrow_mut().push(Box::new(observer));
    }

    pub fn select_only(&self, message: MessageId) {
        self.change(|selector| selector.select_only(message));
    }

    pub fn toggle(&self, message: MessageId) {
        self.change(|selector| selector.toggle(message));
    }

    pub fn extend_to(&self, message: MessageId) {
        self.change(|selector| selector.extend_to(message));
    }

    pub fn extend_over(&self, messages: impl IntoIterator<Item = MessageId>) {
        self.change(|selector| selector.extend_over(messages));
    }

    /// `reach` is recorded now, not re-read when a verb runs: an account reconnecting in between
    /// would silently grow the selection (#811, ADR 0005 Q10).
    pub fn select_all(&self, reach: Reach) {
        self.change(|selector| selector.select_all(reach));
    }

    pub fn reach(&self) -> Reach {
        self.inner.selector.borrow().reach()
    }

    pub fn clear(&self) {
        self.change(Selector::clear);
    }

    pub fn set(&self, selection: Selection) {
        self.change(|selector| selector.set(selection));
    }

    /// The borrow ends before observers are told, so one may read.
    fn change(&self, change: impl FnOnce(&mut Selector) -> bool) {
        let changed = change(&mut self.inner.selector.borrow_mut());
        if changed {
            self.announce();
        }
    }

    fn announce(&self) {
        let selection = self.selection();
        for observer in self.inner.observers.borrow().iter() {
            observer(&selection);
        }
    }
}

/// `total` is passed in because counting is what the `Everything` predicate exists to avoid.
/// `omitted` names accounts a whole-view selection left out (#811); the summary then stops claiming
/// a count, since `total` includes rows the predicate does not name.
pub fn summary(selection: &Selection, total: Option<u32>, omitted: &[String]) -> Option<String> {
    match selection {
        Selection::These(messages) if messages.is_empty() => None,
        Selection::These(messages) => Some(format!("{} selected", count(messages.len() as u32))),
        Selection::Everything { .. } if !omitted.is_empty() => Some(format!(
            "All selected, except {}",
            crate::format::names(omitted)
        )),
        Selection::Everything { except } => match total {
            Some(total) => {
                let selected = total.saturating_sub(except.len() as u32);
                Some(format!("{} selected", count(selected)))
            }
            None => Some("All selected".to_owned()),
        },
    }
}

pub(crate) fn count(value: u32) -> String {
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

/// `None` rows (pages not yet arrived) are skipped, not waited for.
pub fn range(rows: &[Option<MessageId>], from: usize, to: usize) -> Vec<MessageId> {
    let (first, last) = if from <= to { (from, to) } else { (to, from) };
    rows.iter()
        .skip(first)
        .take(last.saturating_sub(first) + 1)
        .flatten()
        .copied()
        .collect()
}

/// A cursor whose own row went stays in place (the row that slid up into it), or on the last row,
/// so `a a a` works down a list (#1687, #1746).
pub fn cursor_after_change(
    cursor: Option<u32>,
    position: u32,
    removed: u32,
    added: u32,
    len: u32,
) -> Option<u32> {
    let cursor = cursor?;
    if len == 0 {
        return None;
    }
    let at = if cursor < position {
        cursor
    } else if cursor >= position + removed {
        cursor - removed + added
    } else {
        cursor
    };
    Some(at.min(len - 1))
}

/// The answer is an offset into `rows`, by place and not slot number: with rows above going too,
/// "the same index" skips a row (#468).
pub fn survivor_below(
    rows: impl IntoIterator<Item = Option<MessageId>>,
    gone: &[MessageId],
) -> Option<usize> {
    let mut rows = rows.into_iter().enumerate();
    let (_, own) = rows.next()?;
    if !own.is_some_and(|own| gone.contains(&own)) {
        return None;
    }
    rows.find(|(_, row)| row.is_none_or(|row| !gone.contains(&row)))
        .map(|(offset, _)| offset)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    fn msg(n: i64) -> MessageId {
        MessageId::new(n)
    }

    #[test]
    fn a_selector_is_send_so_a_controller_can_hold_one() {
        fn send<T: Send>() {}
        send::<Selector>();
    }

    #[test]
    fn a_selector_says_whether_anything_changed() {
        let mut selector = Selector::default();
        assert!(selector.toggle(msg(1)), "x on an unselected row selects it");
        assert_eq!(selector.anchor(), Some(msg(1)));
        assert!(selector.extend_to(msg(2)), "J adds the next row");
        assert_eq!(
            selector.anchor(),
            Some(msg(1)),
            "extending keeps the anchor"
        );
        assert!(!selector.extend_to(msg(2)), "already in: nothing changed");
        assert!(selector.toggle(msg(1)), "x again takes it out");
        assert!(!selector.contains(msg(1)));
        assert!(selector.clear());
        assert!(selector.is_empty());
        assert_eq!(selector.anchor(), None, "clearing drops the anchor");
        assert!(!selector.clear(), "clearing nothing changes nothing");
    }

    #[test]
    fn select_all_is_a_predicate_with_its_reach() {
        let mut selector = Selector::default();
        let reach = Reach {
            accounts: vec![AccountId::new(1)],
            omitted: vec!["Work".to_owned()],
        };
        assert!(selector.select_all(reach.clone()));
        assert!(matches!(selector.selection(), Selection::Everything { .. }));
        assert_eq!(selector.reach(), reach);
        assert!(selector.select_only(msg(3)));
        assert_eq!(
            selector.reach(),
            Reach::default(),
            "a named selection reaches nothing"
        );
    }

    #[test]
    fn the_observable_state_announces_only_changes() {
        let state = SelectionState::new();
        let heard = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = std::rc::Rc::clone(&heard);
        state.connect_changed(move |_| counter.set(counter.get() + 1));
        state.toggle(msg(1));
        state.extend_to(msg(1));
        state.clear();
        state.clear();
        assert_eq!(
            heard.get(),
            2,
            "toggle and the first clear; the repeats were no change"
        );
    }

    use super::*;

    fn id(value: i64) -> MessageId {
        MessageId::new(value)
    }

    #[test]
    fn the_cursor_goes_to_the_first_survivor_below_its_own_row() {
        let rows = |ids: &[i64]| ids.iter().map(|n| Some(id(*n))).collect::<Vec<_>>();
        let gone = [id(1), id(3), id(4)];
        assert_eq!(survivor_below(rows(&[4, 5, 6]), &gone), Some(1));
        assert_eq!(survivor_below(rows(&[3, 4, 7]), &gone), Some(2));
        assert_eq!(survivor_below(rows(&[2, 3, 7]), &gone), None);
        assert_eq!(survivor_below(rows(&[4]), &gone), None);
        assert_eq!(survivor_below(rows(&[3, 4]), &gone), None);
        assert_eq!(survivor_below([Some(id(3)), None], &gone), Some(1));
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
        state.select_all(Reach::default());

        assert!(state.selection().is_everything());
        assert_eq!(state.selection().ids(), None, "nothing was materialised");
        assert!(state.contains(id(98_765)), "including rows never loaded");
    }

    #[test]
    fn taking_a_row_out_of_everything_edits_the_exceptions() {
        let state = SelectionState::new();
        state.select_all(Reach::default());

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
        assert_eq!(summary(&Selection::default(), Some(40), &[]), None);
    }

    #[test]
    fn a_named_selection_counts_itself() {
        let selection = Selection::These(vec![id(1), id(2), id(3)]);

        assert_eq!(
            summary(&selection, None, &[]).as_deref(),
            Some("3 selected")
        );
    }

    #[test]
    fn a_predicate_counts_from_the_total_the_store_already_knows() {
        let selection = Selection::Everything {
            except: vec![id(1), id(2)],
        };

        assert_eq!(
            summary(&selection, Some(50_000), &[]).as_deref(),
            Some("49,998 selected"),
            "counted by arithmetic, never by walking the mailbox"
        );
    }

    #[test]
    fn a_predicate_with_no_total_is_described_rather_than_counted() {
        let selection = Selection::Everything { except: Vec::new() };

        assert_eq!(
            summary(&selection, None, &[]).as_deref(),
            Some("All selected")
        );
    }

    #[test]
    fn the_reach_is_fixed_when_the_gesture_is_made_not_when_it_is_read() {
        // The accounts are the ones the view could show when pressed; an account coming back must
        // not widen the selection.
        let state = SelectionState::new();

        state.select_all(Reach {
            accounts: vec![AccountId::new(1)],
            omitted: vec!["Personal".to_owned()],
        });

        assert_eq!(state.reach().accounts, vec![AccountId::new(1)]);
        assert_eq!(state.reach().omitted, vec!["Personal".to_owned()]);
    }

    #[test]
    fn a_selection_that_names_its_rows_has_no_reach_to_disclose() {
        let state = SelectionState::new();
        state.select_all(Reach {
            accounts: vec![AccountId::new(1)],
            omitted: vec!["Personal".to_owned()],
        });

        state.select_only(id(4));

        assert_eq!(state.reach(), Reach::default());
    }

    #[test]
    fn a_predicate_that_left_an_account_out_names_it_instead_of_counting() {
        // #811: the row count includes an unreachable account's rows that the selection does not.
        let selection = Selection::Everything { except: Vec::new() };

        assert_eq!(
            summary(&selection, Some(50_000), &["Personal".to_owned()]).as_deref(),
            Some("All selected, except Personal"),
        );
    }

    #[test]
    fn every_account_left_out_is_named() {
        let selection = Selection::Everything { except: Vec::new() };

        assert_eq!(
            summary(
                &selection,
                Some(50_000),
                &["Personal".to_owned(), "Work".to_owned()]
            )
            .as_deref(),
            Some("All selected, except Personal and Work"),
        );
    }

    #[test]
    fn a_named_selection_is_never_partial_however_many_accounts_are_away() {
        let selection = Selection::These(vec![id(1), id(2)]);

        assert_eq!(
            summary(&selection, Some(40), &["Personal".to_owned()]).as_deref(),
            Some("2 selected"),
        );
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

    #[test]
    fn the_cursor_on_a_removed_row_stays_on_the_row_that_slid_into_it() {
        assert_eq!(cursor_after_change(Some(1), 1, 1, 0, 35), Some(1));
    }

    #[test]
    fn the_cursor_on_the_removed_last_row_goes_to_the_new_last() {
        assert_eq!(cursor_after_change(Some(4), 4, 1, 0, 4), Some(3));
    }

    #[test]
    fn a_cursor_before_or_after_the_change_keeps_its_row() {
        assert_eq!(cursor_after_change(Some(0), 2, 1, 0, 9), Some(0));
        assert_eq!(cursor_after_change(Some(5), 2, 1, 0, 9), Some(4));
        assert_eq!(cursor_after_change(Some(5), 0, 0, 2, 12), Some(7));
    }

    #[test]
    fn a_rewritten_list_keeps_the_cursor_at_its_place() {
        assert_eq!(cursor_after_change(Some(1), 0, 36, 35, 35), Some(1));
    }

    #[test]
    fn no_cursor_stays_none_and_an_emptied_list_has_none() {
        assert_eq!(cursor_after_change(None, 0, 1, 0, 5), None);
        assert_eq!(cursor_after_change(Some(0), 0, 1, 0, 0), None);
    }
}
