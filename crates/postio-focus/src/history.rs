//! Back and forward between the inbox and search results, as a browser moves between pages: a new
//! search is a visit that drops what was ahead. The inbox's entry carries nothing (the list keeps
//! its own cursor and selection); a results entry carries what the view cannot be asked for again.

use postio_model::MessageId;
use postio_search::results::{ConversationOrder, ResultsTab};

/// The oldest is the least likely to be wanted back.
pub(crate) const LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    Inbox,
    Results(Snapshot),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) query: String,
    pub(crate) tab: ResultsTab,
    pub(crate) order: ConversationOrder,
    pub(crate) cursor: Option<u64>,
    pub(crate) checked: Vec<(MessageId, Option<postio_model::ThreadId>)>,
    pub(crate) all: bool,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    back: Vec<Entry>,
    forward: Vec<Entry>,
}

impl History {
    pub(crate) fn visit(&mut self, leaving: Entry) {
        push(&mut self.back, leaving);
        self.forward.clear();
    }

    pub(crate) fn back(&mut self, current: Entry) -> Option<Entry> {
        let to = self.back.pop()?;
        push(&mut self.forward, current);
        Some(to)
    }

    pub(crate) fn forward(&mut self, current: Entry) -> Option<Entry> {
        let to = self.forward.pop()?;
        push(&mut self.back, current);
        Some(to)
    }

    /// Esc's last rung: the inbox, with `current` ahead of it so ⌘] comes back; the searches
    /// between are let go unless the inbox is right behind.
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

fn push(stack: &mut Vec<Entry>, entry: Entry) {
    stack.push(entry);
    if stack.len() > LIMIT {
        stack.remove(0);
    }
}
