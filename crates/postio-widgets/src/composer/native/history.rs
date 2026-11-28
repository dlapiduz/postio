//! The draft's undo is the document's history (ADR 0004 Q5), never the buffer's; the surface
//! owns only the clock that ends a typing run (research R6).

use std::time::Instant;

use postio_body::{Applied, EditStep, Selection};

use super::Inner;

impl Inner {
    pub(super) fn record(&self, applied: &Applied, before: Selection) {
        let step = EditStep::of(applied, &self.document.borrow(), before);
        let now = Instant::now();
        let mut history = self.history.borrow_mut();
        if self
            .last_edit
            .get()
            .is_none_or(|last| now.duration_since(last) >= self.coalesce)
        {
            history.seal();
        }
        history.record(step, applied.join);
        self.last_edit.set(Some(now));
    }

    pub(super) fn step(&self, back: bool) {
        if self.composing() {
            return;
        }
        let undone = {
            let mut document = self.document.borrow_mut();
            let mut history = self.history.borrow_mut();
            if back {
                history.undo(&mut document)
            } else {
                history.redo(&mut document)
            }
        };
        let Some(undone) = undone else {
            return;
        };
        self.last_edit.set(None);
        self.repaint(undone.changed);
        self.place(&undone.selection);
        *self.selection.borrow_mut() = undone.selection;
        self.notify();
    }
}
