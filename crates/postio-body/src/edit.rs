//! Editing undo, defined over the document rather than over a widget.
//!
//! # Two stacks that must not meet
//!
//! `postio_core::undo` is the **mail** undo: an entry carries its inverse as
//! commands, coalesces a burst into one unit, expires, and is bound to `u`.
//! Archiving a thread goes on it.
//!
//! Text editing undo is none of those things. It is per-typing-run,
//! unbounded within a draft, has no remote half, and dies with the composer.
//! It is `Ctrl+Z`. The command registry already keeps the two apart —
//! `Context::Composer` does not bind the mail verbs — and this is the other
//! half of that separation.
//!
//! # Why not the widget's own
//!
//! Because a `GtkTextBuffer` undo step and a `contenteditable` undo step are
//! not the same thing, so two frontends taking their toolkit's free undo
//! would disagree about what one `Ctrl+Z` does. A step here is a change to a
//! [`Document`], which every surface can produce. See ADR 0004 Q5.
//!
//! # Snapshots, not diffs
//!
//! An [`EditStep`] holds the document before and after. A compact delta would
//! be less memory and is a straightforward later optimisation; what matters
//! architecturally is that the step is defined over the *document*, and that
//! is true either way. Mail bodies are small — a long message is a few
//! kilobytes — so this is not the place to spend complexity first.

use crate::document::Document;

/// One undoable change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditStep {
    /// The document before the change.
    pub before: Document,
    /// The document after it.
    pub after: Document,
}

/// The composer's editing history.
///
/// Bounded, because a draft edited all afternoon should not grow without
/// limit; [`EditHistory::DEPTH`] steps is far more than anyone walks back
/// through by hand, and dropping the oldest is the right thing to lose.
#[derive(Clone, Debug, Default)]
pub struct EditHistory {
    done: Vec<EditStep>,
    undone: Vec<EditStep>,
}

impl EditHistory {
    /// How many steps are kept.
    pub const DEPTH: usize = 200;

    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a change.
    ///
    /// A no-op change is not a step: without this, every autosave tick and
    /// every focus change that re-read the buffer would put an entry on the
    /// stack, and `Ctrl+Z` would appear to do nothing several times before it
    /// did something.
    ///
    /// Recording also drops the redo stack, which is what every editor does:
    /// once you type after undoing, the branch you undid is gone.
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

    /// Extend the newest step to end at `after`.
    ///
    /// What makes a typing *run* one undo, rather than one undo per
    /// keystroke. The step's `before` is untouched, so undoing still lands
    /// where the run started. Returns false when there is no step to extend.
    pub fn amend(&mut self, after: Document) -> bool {
        match self.done.last_mut() {
            Some(step) => {
                step.after = after;
                true
            }
            None => false,
        }
    }

    /// Step back, returning the document to show.
    pub fn undo(&mut self) -> Option<Document> {
        let step = self.done.pop()?;
        let document = step.before.clone();
        self.undone.push(step);
        Some(document)
    }

    /// Step forward again.
    pub fn redo(&mut self) -> Option<Document> {
        let step = self.undone.pop()?;
        let document = step.after.clone();
        self.done.push(step);
        Some(document)
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Forget everything. The composer closing, or a different draft opening.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}
