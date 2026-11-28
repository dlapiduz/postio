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
