//! The three doors into the buffer (research R4): keys in the capture phase, text through the
//! buffer's `insert-text`, and deletions through `delete-range`. Each becomes an intent; the
//! buffer itself is never let through, so the patch is its only writer.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_body::{EditIntent, Selection};
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
        if inner.selection.borrow().preedit.is_some() {
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
            inner.typed(location.offset() as u32, text);
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
                inner.dispatch_at(range, EditIntent::DeleteForward(postio_body::Unit::Char));
            }
        });

    let weak = Rc::downgrade(inner);
    inner.view.connect_preedit_changed(move |_, preedit| {
        if let Some(inner) = weak.upgrade() {
            inner.selection.borrow_mut().preedit =
                (!preedit.is_empty()).then(|| preedit.to_owned());
        }
    });
}

impl Inner {
    /// True when the key did something; a Tab that did nothing lets the focus move.
    fn key(&self, edit: KeyEdit) -> bool {
        if edit == KeyEdit::SplitBlock
            && let Some(conversion) = self.conversion(Typed::Enter)
        {
            return self.dispatch(EditIntent::Convert(conversion));
        }
        self.dispatch(edit.intent())
    }

    fn typed(&self, offset: u32, text: &str) {
        let at = self.projection.borrow().position_of(offset);
        let now = self.selection_now();
        let selection = if now.is_collapsed() && now.head == at {
            now
        } else {
            Selection::caret(at)
        };
        let intent = if selection.preedit.is_some() {
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
        let text = self.buffer.text(&from, &to, true);
        conversion_before(&text, selection.head.offset as usize, typed)
    }
}
