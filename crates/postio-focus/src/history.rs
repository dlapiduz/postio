//! Back and forward between the inbox and search results (spec 010 D17,
//! research R9).
//!
//! The main window has two modes, the inbox and a search's results, and
//! moves between them the way a browser moves between pages: a new search
//! is a visit, which drops whatever was ahead; back and forward step
//! through what was visited, ⌘[ and ⌘] or the trackpad's swipe. Nothing
//! closes on the way (D18): history only says which mode to show and what
//! it held.
//!
//! The inbox's entry carries nothing. The list keeps its own cursor and
//! selection under the results, which never touch them, so going back to it
//! is showing it as it is. A results entry carries what the view cannot be
//! asked for again: its query, tab, sort, the row with the focus ring and
//! the rows checked.

use postio_model::MessageId;
use postio_search::results::{ConversationOrder, ResultsTab};

/// Entries kept each way: a search session of fifty is a long one, and
/// the oldest is the least likely to be wanted back.
pub(crate) const LIMIT: usize = 50;

/// One place the main window has shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    /// The inbox, with its own cursor and selection.
    Inbox,
    /// A search's results, as they were left.
    Results(Snapshot),
}

/// The results view, as it was left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    /// The query, as the field holds it.
    pub(crate) query: String,
    /// The tab.
    pub(crate) tab: ResultsTab,
    /// The sort.
    pub(crate) order: ConversationOrder,
    /// The row with the focus ring.
    pub(crate) cursor: Option<u64>,
    /// The rows checked, by their best message and its conversation; with
    /// `all`, the rows taken back out of the whole match.
    pub(crate) checked: Vec<(MessageId, Option<postio_model::ThreadId>)>,
    /// ⇧X: every conversation the query matches is checked.
    pub(crate) all: bool,
}

/// What was visited behind the place shown, and what is ahead of it.
#[derive(Debug, Default)]
pub(crate) struct History {
    back: Vec<Entry>,
    forward: Vec<Entry>,
}

impl History {
    /// A new place is shown, `leaving` the one before: it goes behind, and
    /// what was ahead is gone.
    pub(crate) fn visit(&mut self, leaving: Entry) {
        push(&mut self.back, leaving);
        self.forward.clear();
    }

    /// Step back from `current`, which goes ahead; the place to show, or
    /// `None` with nothing behind.
    pub(crate) fn back(&mut self, current: Entry) -> Option<Entry> {
        let to = self.back.pop()?;
        push(&mut self.forward, current);
        Some(to)
    }

    /// Step forward from `current`, which goes behind; the place to show,
    /// or `None` with nothing ahead.
    pub(crate) fn forward(&mut self, current: Entry) -> Option<Entry> {
        let to = self.forward.pop()?;
        push(&mut self.back, current);
        Some(to)
    }

    /// Esc's last rung from `current` (D18): the inbox, whatever searches
    /// were run from the results on the way, and `current` ahead of it so
    /// ⌘] comes back to it. Straight back when the inbox is right behind;
    /// otherwise the searches between are let go.
    pub(crate) fn home(&mut self, current: Entry) {
        if self.back.last() == Some(&Entry::Inbox) {
            let _ = self.back(current);
            return;
        }
        while let Some(entry) = self.back.pop() {
            if entry == Entry::Inbox {
                break;
            }
        }
        self.forward.clear();
        push(&mut self.forward, current);
    }
}

/// `entry` on top of `stack`, the oldest let go past [`LIMIT`].
fn push(stack: &mut Vec<Entry>, entry: Entry) {
    stack.push(entry);
    if stack.len() > LIMIT {
        stack.remove(0);
    }
}
