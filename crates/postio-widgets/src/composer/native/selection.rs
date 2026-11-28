//! A bullet, a number and the reply quote have no document position, so a caret the toolkit puts
//! there (a click, Home) is moved to the text the projection says it means.

use std::rc::Rc;

use gtk::prelude::*;

use super::Inner;

pub(super) fn wire(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    inner.buffer.connect_mark_set(move |buffer, iter, mark| {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        if inner.patching.get() {
            return;
        }
        let insert = buffer.get_insert();
        let bound = buffer.selection_bound();
        if *mark != insert && *mark != bound {
            return;
        }
        let offset = iter.offset() as u32;
        let landed = {
            let projection = inner.projection.borrow();
            projection.offset_of(&projection.position_of(offset))
        };
        if landed == offset {
            return;
        }
        inner.patching.set(true);
        let to = buffer.iter_at_offset(landed as i32);
        if *mark == insert && buffer.iter_at_mark(&bound).offset() as u32 == offset {
            buffer.place_cursor(&to);
        } else {
            buffer.move_mark(mark, &to);
        }
        inner.patching.set(false);
    });
}
