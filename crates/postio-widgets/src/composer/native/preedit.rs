//! An input method's composition is drawn by the view and never reaches the buffer (research R3);
//! the surface only records that one is open, which is what `apply` refuses intents on.

use std::rc::Rc;

use gtk::prelude::*;

use super::Inner;

pub(super) fn wire(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.view.connect_preedit_changed(move |_, preedit| {
        if let Some(inner) = weak.upgrade() {
            inner.selection.borrow_mut().preedit =
                (!preedit.is_empty()).then(|| preedit.to_owned());
        }
    });
}

impl Inner {
    pub(super) fn composing(&self) -> bool {
        self.selection.borrow().preedit.is_some()
    }
}
