//! Nothing mutates a composer's text but the patch of `apply`'s result (ADR 0039).

mod blocks;
mod convert;
pub(crate) mod flat;
mod lists;
mod marks;
mod paste;
mod text;
pub(crate) mod tree;

pub use convert::{Conversion, ConversionKind};
pub use flat::MarkSet;
pub use marks::marks_in_force;
pub use paste::slice;

use crate::document::{Block, ContentId, Document, HeadingLevel, Href};
use crate::position::{DocPosition, Selection};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditIntent {
    Insert(String),
    DeleteBackward(Unit),
    DeleteForward(Unit),
    SplitBlock,
    InsertBreak,
    ToggleMark(Mark),
    SetLink(Option<Href>),
    InsertImage { content_id: ContentId, alt: String },
    ToggleBlock(BlockKind),
    Indent,
    Outdent,
    InsertRule,
    Paste(Document),
    Convert(Conversion),
    CommitPreedit(String),
    CancelPreedit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading(HeadingLevel),
    BulletList,
    NumberedList,
    TaskList,
    Quote,
    CodeBlock,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub selection: Selection,
    pub changed: BlockRange,
    pub join: Join,
    pub replaced: Vec<Block>,
}

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
    pub fn edit(&mut self, selection: &Selection, intent: EditIntent) -> Applied {
        let unchanged = |selection: &Selection| Applied {
            selection: selection.clone(),
            changed: BlockRange::default(),
            join: Join::New,
            replaced: Vec::new(),
        };
        let composing = selection.preedit.is_some();
        let preedit_intent = matches!(
            intent,
            EditIntent::CommitPreedit(_) | EditIntent::CancelPreedit
        );
        if !selection.is_valid_in(self) || composing && !preedit_intent {
            return unchanged(selection);
        }
        let (lo, hi) = reach(self, selection, &intent);
        let before = self.blocks[lo..hi].to_vec();
        let old_len = self.blocks.len();
        let outcome = run(&mut self.blocks, selection, intent);
        let new_hi = (hi + self.blocks.len()).saturating_sub(old_len);
        let Some(outcome) = outcome else {
            // A rule may refuse after a first step, so a refusal restores what it reached.
            self.blocks.splice(lo..new_hi, before);
            return unchanged(selection);
        };
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

fn without_preedit(selection: &Selection) -> Selection {
    Selection {
        preedit: None,
        ..selection.clone()
    }
}

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
        EditIntent::ToggleBlock(kind) => blocks::toggle(blocks, selection, kind),
        EditIntent::Indent => blocks::indent(blocks, selection),
        EditIntent::Outdent => blocks::outdent(blocks, selection),
        EditIntent::InsertRule => blocks::insert_rule(blocks, selection),
        EditIntent::Paste(pasted) => paste::paste(blocks, selection, pasted),
        EditIntent::Convert(conversion) => convert::convert(blocks, selection, conversion),
        EditIntent::CommitPreedit(text) => {
            let committed = text::insert(blocks, &without_preedit(selection), &text)?;
            Some(Outcome {
                join: Join::New,
                ..committed
            })
        }
        EditIntent::CancelPreedit => Some(Outcome {
            selection: without_preedit(selection),
            join: Join::New,
        }),
    }
}
