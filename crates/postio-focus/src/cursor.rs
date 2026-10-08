//! The cursor, the selection and the has-action filter (research R2,
//! slice 3; contract invariants 1-3).
//!
//! Moved from `postio-gtk`'s window so the Mac's list moves and selects by
//! the same rules. The cursor is where the keyboard is; the selection is
//! what `a` would archive; they are never the same thing (constitution II).
//! The rows themselves stay with each toolkit: the controller reads them
//! through [`Rows`] and says where the cursor goes and what is selected.

use std::collections::HashMap;

use postio_core::CommandId;
use postio_core::state::Selection;
use postio_model::{AccountId, FocusScope, ListScope, MessageId, ThreadId};
use postio_ui::focus_list::FocusRow;
use postio_ui::selection::{Reach, Selector};

use crate::Intent;
use crate::feed::Step;

/// What the controller needs to know about one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFacts {
    /// The row's message: what the cursor stays on and the selection names.
    pub id: MessageId,
    /// Whether it is a digest's row, which no bulk verb reaches.
    pub digest: bool,
    /// The conversations it stands for, every copy (spec 007 T161).
    pub threads: Vec<ThreadId>,
}

impl RowFacts {
    /// What `row` is, to the controller.
    pub fn of(row: &FocusRow) -> Self {
        RowFacts {
            id: row.id(),
            digest: matches!(row, FocusRow::Digest(_)),
            threads: row.threads(),
        }
    }
}

/// The list as the frontend holds it. The controller never keeps a copy of
/// the rows (one resident window, constitution V); it asks.
pub trait Rows {
    /// How many rows the list draws.
    fn len(&self) -> u32;
    /// Whether the list draws no rows.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The row at `position`, once its page has landed.
    fn facts(&self, position: u32) -> Option<RowFacts>;
    /// Where `message`'s row is, when it is in a landed page.
    fn position_of(&self, message: MessageId) -> Option<u32>;
}

/// No list: for inputs that need none.
pub struct NoRows;

impl Rows for NoRows {
    fn len(&self) -> u32 {
        0
    }
    fn facts(&self, _: u32) -> Option<RowFacts> {
        None
    }
    fn position_of(&self, _: MessageId) -> Option<u32> {
        None
    }
}

/// Where the keyboard is, what is selected, and whether `!` is on.
#[derive(Debug, Default)]
pub(crate) struct Cursor {
    /// The cursor's row.
    position: Option<u32>,
    /// The message `!` left the cursor on, to find again when the list lands.
    keep: Option<MessageId>,
    selector: Selector,
    /// The conversations each selected row stands for.
    reach: HashMap<MessageId, Vec<ThreadId>>,
    /// Whether the list is narrowed to rows with a marker (`!`).
    has_action: bool,
    /// The accounts Focus's inbox is made of: what "everything" reaches.
    accounts: Vec<AccountId>,
}

impl Cursor {
    pub(crate) fn position(&self) -> Option<u32> {
        self.position
    }

    pub(crate) fn has_action(&self) -> bool {
        self.has_action
    }

    pub(crate) fn selection(&self) -> Selection {
        self.selector.selection()
    }

    pub(crate) fn reach(&self) -> &HashMap<MessageId, Vec<ThreadId>> {
        &self.reach
    }

    pub(crate) fn accounts(&self) -> &[AccountId] {
        &self.accounts
    }

    /// The cursor's row, once its page has landed.
    pub(crate) fn row(&self, rows: &dyn Rows) -> Option<RowFacts> {
        self.cursor_row(rows)
    }

    /// Put the cursor on `position`.
    pub(crate) fn place(&mut self, position: u32, rows: &dyn Rows) -> Vec<Step> {
        self.to(Some(position), rows)
    }

    pub(crate) fn set_accounts(&mut self, accounts: Vec<AccountId>) {
        self.accounts = accounts;
    }

    /// The list has landed: an opening puts the cursor on its first row
    /// (C30), or back on the message `!` kept; a re-read leaves it.
    pub(crate) fn landed(&mut self, rows: &dyn Rows, opened: bool) -> Vec<Step> {
        if let Some(message) = self.keep.take() {
            return match rows.position_of(message) {
                Some(position) => self.to(Some(position), rows),
                None if !rows.is_empty() => self.to(Some(0), rows),
                None => Vec::new(),
            };
        }
        if opened && !rows.is_empty() {
            return self.to(Some(0), rows);
        }
        Vec::new()
    }

    /// A command the cursor answers, or `None` when it is not one of its.
    pub(crate) fn command(
        &mut self,
        id: CommandId,
        rows: &dyn Rows,
        scope: Option<ListScope>,
        total: u32,
        has_action_count: Option<u32>,
    ) -> Option<Vec<Step>> {
        let steps = match id {
            CommandId::NextMessage => self.by(1, rows),
            CommandId::PrevMessage => self.by(-1, rows),
            CommandId::FirstMessage => self.to(Some(0), rows),
            CommandId::LastMessage => self.to(rows.len().checked_sub(1), rows),
            CommandId::ToggleSelection => match self.cursor_row(rows) {
                Some(row) if !row.digest => {
                    self.reach.insert(row.id, row.threads);
                    let changed = self.selector.toggle(row.id);
                    self.selection_steps(changed, total)
                }
                _ => Vec::new(),
            },
            CommandId::ExtendSelectionDown => self.extend(1, rows, total),
            CommandId::ExtendSelectionUp => self.extend(-1, rows, total),
            CommandId::SelectAll => {
                let changed = self.selector.select_all(Reach {
                    accounts: self.accounts.clone(),
                    omitted: Vec::new(),
                });
                self.selection_steps(changed, total)
            }
            CommandId::ToggleHasAction => self.toggle_has_action(rows, scope, has_action_count),
            _ => return None,
        };
        Some(steps)
    }

    /// Back's last rung: drop the selection; the cursor stays.
    pub(crate) fn clear(&mut self, total: u32) -> Vec<Step> {
        self.reach.clear();
        let changed = self.selector.clear();
        self.selection_steps(changed, total)
    }

    /// `!`: narrow the inbox to the rows with a marker, or back (spec 007
    /// FR-017). The selection goes, since what it named may not be shown;
    /// the cursor's message is kept, to find again when the list lands.
    fn toggle_has_action(
        &mut self,
        rows: &dyn Rows,
        scope: Option<ListScope>,
        has_action_count: Option<u32>,
    ) -> Vec<Step> {
        // It narrows the inbox: a place has no marked rows of its own to
        // narrow to, and the toggle is not offered there.
        let in_inbox = matches!(
            scope,
            None | Some(ListScope::Focus(FocusScope::Inbox | FocusScope::HasAction))
        );
        if !in_inbox {
            return Vec::new();
        }
        self.has_action = !self.has_action;
        self.keep = self.cursor_row(rows).map(|row| row.id);
        let mut steps = self.clear(0);
        steps.push(Step::Show(Intent::SingleHeading(self.has_action.then(
            || postio_ui::focus_row::has_action_label(has_action_count),
        ))));
        steps.push(Step::Open(ListScope::Focus(if self.has_action {
            FocusScope::HasAction
        } else {
            FocusScope::Inbox
        })));
        steps
    }

    /// `J`/`K`: take the cursor's row into the selection and the next one
    /// with it, moving the cursor onto it. A digest row is walked over, not
    /// taken in: no bulk verb reaches it, and `x` refuses it the same way.
    fn extend(&mut self, by: i32, rows: &dyn Rows, total: u32) -> Vec<Step> {
        let mut changed = self.take_cursor_row(rows);
        let mut steps = self.by(by, rows);
        changed |= self.take_cursor_row(rows);
        steps.extend(self.selection_steps(changed, total));
        steps
    }

    fn take_cursor_row(&mut self, rows: &dyn Rows) -> bool {
        match self.cursor_row(rows) {
            Some(row) if !row.digest => {
                self.reach.insert(row.id, row.threads);
                self.selector.extend_to(row.id)
            }
            _ => false,
        }
    }

    fn cursor_row(&self, rows: &dyn Rows) -> Option<RowFacts> {
        rows.facts(self.position?)
    }

    /// Move the cursor `by` rows, and nothing else: nothing opens, nothing
    /// is marked read, the selection stays as it was (spec 007 FR-016).
    fn by(&mut self, by: i32, rows: &dyn Rows) -> Vec<Step> {
        let len = rows.len();
        if len == 0 {
            return Vec::new();
        }
        let next = match self.position {
            None => 0,
            Some(at) => (i64::from(at) + i64::from(by)).clamp(0, i64::from(len) - 1) as u32,
        };
        self.to(Some(next), rows)
    }

    /// Put the cursor on `position`, and have the frontend bring it into
    /// view; a position off the list moves nothing.
    fn to(&mut self, position: Option<u32>, rows: &dyn Rows) -> Vec<Step> {
        match position {
            Some(position) if position < rows.len() => {
                self.position = Some(position);
                vec![Step::Show(Intent::Cursor {
                    position,
                    to_top: position == 0,
                })]
            }
            _ => Vec::new(),
        }
    }

    fn selection_steps(&self, changed: bool, total: u32) -> Vec<Step> {
        if !changed {
            return Vec::new();
        }
        let selection = self.selector.selection();
        let summary = postio_ui::selection::summary(&selection, Some(total), &[]);
        vec![Step::Show(Intent::Selection { selection, summary })]
    }
}
