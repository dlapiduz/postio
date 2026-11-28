//! Editing undo over the document, not a widget: a step is a `Document` change every surface can
//! produce (ADR 0004 Q5).
//! Separate from `postio_core::undo`, the mail undo bound to `u`; this one is `Ctrl+Z`, per typing
//! run, with no remote half.

use crate::document::Document;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditStep {
    pub before: Document,
    pub after: Document,
}

/// Bounded to `DEPTH` steps; the oldest is dropped.
#[derive(Clone, Debug, Default)]
pub struct EditHistory {
    done: Vec<EditStep>,
    undone: Vec<EditStep>,
}

impl EditHistory {
    pub const DEPTH: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    /// A no-op change is not a step, or autosave ticks and focus changes would fill the stack.
    /// Recording drops the redo stack.
    pub fn record(&mut self, before: Document, after: Document) {
        if before == after {
            return;
        }
        self.undone.clear();
        self.done.push(EditStep { before, after });
        if self.done.len() > Self::DEPTH {
            self.done.remove(0);
        }
    }

    /// Extends the newest step so a typing run is one undo; `before` is untouched. Returns false
    /// when there is no step.
    pub fn amend(&mut self, after: Document) -> bool {
        match self.done.last_mut() {
            Some(step) => {
                step.after = after;
                true
            }
            None => false,
        }
    }

    pub fn undo(&mut self) -> Option<Document> {
        let step = self.done.pop()?;
        let document = step.before.clone();
        self.undone.push(step);
        Some(document)
    }

    pub fn redo(&mut self) -> Option<Document> {
        let step = self.undone.pop()?;
        let document = step.after.clone();
        self.done.push(step);
        Some(document)
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
    }
}
