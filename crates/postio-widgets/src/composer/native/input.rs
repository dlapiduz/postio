//! The three doors into the buffer (research R4): keys in the capture phase, text through the
//! buffer's `insert-text`, and deletions through `delete-range`. Each becomes an intent; the
//! buffer itself is never let through, so the patch is its only writer.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_body::{DocPosition, EditIntent, Selection};
use postio_ui::editor::keys::{KeyEdit, key_edit};
use postio_ui::editor::markdown::{Typed, conversion_before};

use super::Inner;

pub(super) fn wire(inner: &Rc<Inner>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(inner);
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(inner) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        // An open composition owns its keys: Return or Escape is the input method's to commit.
        if inner.composing() {
            return glib::Propagation::Proceed;
        }
        let key = match key {
            gtk::gdk::Key::ISO_Left_Tab => gtk::gdk::Key::Tab,
            gtk::gdk::Key::KP_Enter | gtk::gdk::Key::ISO_Enter => gtk::gdk::Key::Return,
            other => other,
        };
        let Some(edit) = crate::keys::chord(key, state).and_then(|chord| key_edit(&chord)) else {
            return glib::Propagation::Proceed;
        };
        if inner.key(edit) || !edit.falls_through() {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    inner.view.add_controller(keys);

    let weak = Rc::downgrade(inner);
    inner
        .buffer
        .connect_insert_text(move |buffer, location, text| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if inner.patching.get() {
                return;
            }
            buffer.stop_signal_emission_by_name("insert-text");
            let at = inner
                .projection
                .borrow()
                .position_of(location.offset() as u32);
            let committed = inner.composing();
            enqueue(
                &inner,
                Pending::Typed {
                    at,
                    text: text.to_owned(),
                    committed,
                },
            );
        });

    let weak = Rc::downgrade(inner);
    inner
        .buffer
        .connect_delete_range(move |buffer, start, end| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if inner.patching.get() {
                return;
            }
            buffer.stop_signal_emission_by_name("delete-range");
            let range = {
                let projection = inner.projection.borrow();
                Selection::range(
                    projection.position_of(start.offset() as u32),
                    projection.position_of(end.offset() as u32),
                )
            };
            if !range.is_collapsed() {
                enqueue(&inner, Pending::Deleted(range));
            }
        });

    super::preedit::wire(inner);
}

/// Text and deletions the view makes arrive inside its own handlers, an input method's commit
/// among them: writing the buffer there re-enters the input method, which crashed GTK. So they
/// queue, in order, and run before the next event.
pub(super) enum Pending {
    Typed {
        at: DocPosition,
        text: String,
        committed: bool,
    },
    Deleted(Selection),
}

fn enqueue(inner: &Rc<Inner>, item: Pending) {
    let mut pending = inner.pending.borrow_mut();
    if let (
        Some(Pending::Typed { at, text, .. }),
        Pending::Typed {
            at: next,
            text: more,
            ..
        },
    ) = (pending.last_mut(), &item)
        && at == next
    {
        text.push_str(more);
        return;
    }
    let first = pending.is_empty();
    pending.push(item);
    drop(pending);
    if first {
        let weak = Rc::downgrade(inner);
        glib::idle_add_local_full(glib::Priority::HIGH, move || {
            if let Some(inner) = weak.upgrade() {
                inner.flush();
            }
            glib::ControlFlow::Break
        });
    }
}

impl Inner {
    pub(super) fn flush(&self) {
        let items = std::mem::take(&mut *self.pending.borrow_mut());
        for item in items {
            match item {
                Pending::Typed {
                    at,
                    text,
                    committed,
                } => self.typed(at, &text, committed),
                Pending::Deleted(range) => {
                    self.dispatch_at(range, EditIntent::DeleteForward(postio_body::Unit::Char));
                }
            }
        }
    }

    /// True when the key did something; a Tab that did nothing lets the focus move.
    fn key(&self, edit: KeyEdit) -> bool {
        if edit == KeyEdit::SplitBlock
            && let Some(conversion) = self.conversion(Typed::Enter)
        {
            return self.dispatch(EditIntent::Convert(conversion));
        }
        self.dispatch(edit.intent())
    }

    fn typed(&self, at: DocPosition, text: &str, committed: bool) {
        let now = self.selection_now();
        let selection = if now.is_collapsed() && now.head == at {
            now
        } else {
            Selection::caret(at)
        };
        let intent = if committed {
            EditIntent::CommitPreedit(text.to_owned())
        } else {
            EditIntent::Insert(text.to_owned())
        };
        self.dispatch_at(selection, intent);
        if let Some(conversion) = self.conversion(Typed::Text) {
            self.dispatch(EditIntent::Convert(conversion));
        }
    }

    /// The caret's line as the matcher reads it, from the buffer: it is the projection of the line.
    fn conversion(&self, typed: Typed) -> Option<postio_body::Conversion> {
        let selection = self.selection_now();
        if !selection.is_collapsed() {
            return None;
        }
        let projection = self.projection.borrow();
        let line = projection
            .lines_in(projection.offset_of(&selection.head)..projection.offset_of(&selection.head))
            .find(|line| line.path == selection.head.path)?;
        let from = self.buffer.iter_at_offset(line.text_start as i32);
        let to = self
            .buffer
            .iter_at_offset((line.text_start + line.len) as i32);
        let text = self.buffer.slice(&from, &to, true);
        conversion_before(&text, selection.head.offset as usize, typed)
    }
}
