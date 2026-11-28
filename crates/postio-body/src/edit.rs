//! Editing undo over the document, not a widget: a step is a `Document` change every surface can
//! produce (ADR 0004 Q5), held as the top-level blocks it replaced so a step costs what it changed.
//! Separate from `postio_core::undo`, the mail undo bound to `u`; this one is `Ctrl+Z`.

use crate::document::{Block, Document};
use crate::edit_ops::{Applied, BlockRange, Join};
use crate::position::Selection;

/// Blocks `at..at + before.len()` became `at..at + after.len()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditStep {
    pub at: u32,
    pub before: Vec<Block>,
    pub after: Vec<Block>,
    pub selection_before: Selection,
    pub selection_after: Selection,
}

impl EditStep {
    /// The step an in-place [`Document::edit`] made; `document` is the edited one.
    pub fn of(applied: &Applied, document: &Document, selection_before: Selection) -> Self {
        let range = applied.changed;
        EditStep {
            at: range.start,
            before: applied.replaced.clone(),
            after: document.blocks[range.start as usize..range.new_end as usize].to_vec(),
            selection_before,
            selection_after: applied.selection.clone(),
        }
    }

    /// For a surface that sees whole documents: the blocks between their common ends.
    pub fn between(
        before: &Document,
        after: &Document,
        selection_before: Selection,
        selection_after: Selection,
    ) -> Self {
        let (old, new) = (&before.blocks, &after.blocks);
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        EditStep {
            at: prefix as u32,
            before: old[prefix..old.len() - suffix].to_vec(),
            after: new[prefix..new.len() - suffix].to_vec(),
            selection_before,
            selection_after,
        }
    }

    fn is_empty(&self) -> bool {
        self.before == self.after
    }
}

/// What an undo or redo did to the document it was handed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undone {
    pub selection: Selection,
    pub changed: BlockRange,
}

/// Bounded to `DEPTH` steps; the oldest is dropped.
#[derive(Clone, Debug, Default)]
pub struct EditHistory {
    done: Vec<EditStep>,
    undone: Vec<EditStep>,
    open: bool,
}

impl EditHistory {
    pub const DEPTH: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    /// `Join::Run` extends the newest step while the run is open and the step covers the same
    /// blocks; anything else starts a step. A no-op is not a step, or autosave ticks would fill the
    /// stack. Recording drops the redo stack.
    pub fn record(&mut self, step: EditStep, join: Join) {
        if step.is_empty() {
            return;
        }
        self.undone.clear();
        let extends = self.open
            && join == Join::Run
            && self
                .done
                .last()
                .is_some_and(|last| last.at == step.at && last.after.len() == step.before.len());
        match self.done.last_mut() {
            Some(last) if extends => {
                last.after = step.after;
                last.selection_after = step.selection_after;
            }
            _ => {
                self.done.push(step);
                if self.done.len() > Self::DEPTH {
                    self.done.remove(0);
                }
            }
        }
        self.open = join == Join::Run;
    }

    /// Ends the typing run, so the next `Join::Run` starts a step: the surface's clock says when.
    pub fn seal(&mut self) {
        self.open = false;
    }

    pub fn undo(&mut self, document: &mut Document) -> Option<Undone> {
        let step = self.done.pop()?;
        self.open = false;
        let undone = swap(
            document,
            step.at,
            step.after.len(),
            &step.before,
            &step.selection_before,
        );
        self.undone.push(step);
        Some(undone)
    }

    pub fn redo(&mut self, document: &mut Document) -> Option<Undone> {
        let step = self.undone.pop()?;
        self.open = false;
        let redone = swap(
            document,
            step.at,
            step.before.len(),
            &step.after,
            &step.selection_after,
        );
        self.done.push(step);
        Some(redone)
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.open = false;
    }
}

fn swap(
    document: &mut Document,
    at: u32,
    len: usize,
    with: &[Block],
    selection: &Selection,
) -> Undone {
    let start = (at as usize).min(document.blocks.len());
    let end = (start + len).min(document.blocks.len());
    document.blocks.splice(start..end, with.iter().cloned());
    Undone {
        selection: selection.clone(),
        changed: BlockRange {
            start: start as u32,
            old_end: end as u32,
            new_end: (start + with.len()) as u32,
        },
    }
}
