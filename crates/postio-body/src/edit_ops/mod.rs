//! The editing algebra: every change a person makes to a draft, as a function of the document, the
//! selection and an intent. Nothing mutates a composer's text but the patch of `apply`'s result
//! (ADR 0039); the surfaces decide nothing this module can.

pub(crate) mod flat;
mod lists;
mod marks;
mod text;
pub(crate) mod tree;

pub use flat::MarkSet;

use crate::document::{Block, ContentId, Document, Href};
use crate::position::{DocPosition, Selection};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditIntent {
    /// Typed or committed text; a newline is a line break.
    Insert(String),
    DeleteBackward(Unit),
    DeleteForward(Unit),
    /// Enter.
    SplitBlock,
    /// Shift+Enter.
    InsertBreak,
    /// Over a range, on unless every character has it; at a caret, for the next typed text.
    ToggleMark(Mark),
    /// `None` unlinks.
    SetLink(Option<Href>),
    InsertImage {
        content_id: ContentId,
        alt: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Strong,
    Emphasis,
    Strikethrough,
    Code,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Char,
    Word,
    Line,
}

/// Top-level blocks `start..old_end` of the input became `start..new_end` of the output; every
/// block outside is equal to the input's. Empty when nothing changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockRange {
    pub start: u32,
    pub old_end: u32,
    pub new_end: u32,
}

impl BlockRange {
    pub fn is_empty(&self) -> bool {
        self.start == self.old_end && self.start == self.new_end
    }
}

/// Whether the step may extend the open undo step. The surface owns the clock that ends a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Run,
    New,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edited {
    pub document: Document,
    pub selection: Selection,
    pub changed: BlockRange,
    pub join: Join,
}

impl Edited {
    pub fn unchanged(document: Document, selection: Selection) -> Self {
        Edited {
            document,
            selection,
            changed: BlockRange::default(),
            join: Join::New,
        }
    }
}

/// An edit made in place: what [`apply`] reports, and the blocks it replaced, for the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub selection: Selection,
    pub changed: BlockRange,
    pub join: Join,
    pub replaced: Vec<Block>,
}

/// Pure and total: an intent that does not apply returns the input with an empty range.
pub fn apply(document: &Document, selection: &Selection, intent: EditIntent) -> Edited {
    let mut next = document.clone();
    let applied = next.edit(selection, intent);
    Edited {
        document: next,
        selection: applied.selection,
        changed: applied.changed,
        join: applied.join,
    }
}

pub(crate) struct Outcome {
    pub selection: Selection,
    pub join: Join,
}

impl Outcome {
    pub fn at(position: DocPosition, join: Join) -> Option<Self> {
        Some(Outcome {
            selection: Selection::caret(position),
            join,
        })
    }
}

impl Document {
    /// [`apply`] without copying the blocks it leaves alone, for a surface on a long draft.
    pub fn edit(&mut self, selection: &Selection, intent: EditIntent) -> Applied {
        let unchanged = |selection: &Selection| Applied {
            selection: selection.clone(),
            changed: BlockRange::default(),
            join: Join::New,
            replaced: Vec::new(),
        };
        if !selection.is_valid_in(self) {
            return unchanged(selection);
        }
        let (lo, hi) = reach(self, selection, &intent);
        let before = self.blocks[lo..hi].to_vec();
        let old_len = self.blocks.len();
        let Some(outcome) = run(&mut self.blocks, selection, intent) else {
            return unchanged(selection);
        };
        let new_hi = (hi + self.blocks.len()).saturating_sub(old_len);
        let after = &self.blocks[lo..new_hi];
        let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
        let room = before.len().min(after.len()) - prefix;
        let suffix = before
            .iter()
            .rev()
            .zip(after.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let mut replaced = before;
        replaced.truncate(replaced.len() - suffix);
        replaced.drain(..prefix);
        Applied {
            selection: outcome.selection,
            changed: BlockRange {
                start: (lo + prefix) as u32,
                old_end: (hi - suffix) as u32,
                new_end: (new_hi - suffix) as u32,
            },
            join: outcome.join,
            replaced,
        }
    }
}

/// The top-level blocks an intent may touch: the selection's, and a neighbour for a merge.
fn reach(document: &Document, selection: &Selection, intent: &EditIntent) -> (usize, usize) {
    let (start, end) = (selection.start(), selection.end());
    let mut lo = start.top();
    let mut hi = end.top() + 1;
    if selection.is_collapsed() {
        match intent {
            EditIntent::DeleteBackward(_) if start.offset == 0 => lo = lo.saturating_sub(1),
            EditIntent::DeleteForward(_) => hi += 1,
            _ => {}
        }
    }
    (lo, hi.min(document.blocks.len()))
}

fn run(blocks: &mut Vec<Block>, selection: &Selection, intent: EditIntent) -> Option<Outcome> {
    match intent {
        EditIntent::Insert(typed) => text::insert(blocks, selection, &typed),
        EditIntent::DeleteBackward(unit) => text::delete_backward(blocks, selection, unit),
        EditIntent::DeleteForward(unit) => text::delete_forward(blocks, selection, unit),
        EditIntent::SplitBlock => text::split_block(blocks, selection),
        EditIntent::InsertBreak => text::insert_break(blocks, selection),
        EditIntent::ToggleMark(mark) => marks::toggle(blocks, selection, mark),
        EditIntent::SetLink(href) => marks::set_link(blocks, selection, href),
        EditIntent::InsertImage { content_id, alt } => {
            marks::insert_image(blocks, selection, content_id, alt)
        }
    }
}
