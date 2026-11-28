//! The reply quote (Q2 = A): a read-only plain-text range, tinted, one unit the caret cannot
//! enter, folded to its first lines until the person opens it.

use std::rc::Rc;

use gtk::prelude::*;
use postio_body::Run;
use postio_body::projection::AtomicKind;

use super::{Inner, NativeSurface};

pub(super) const SHOWN_LINES: usize = 3;

pub(super) fn wire(inner: &Rc<Inner>) {
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(inner);
    click.connect_released(move |_, _, x, y| {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let (bx, by) =
            inner
                .view
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let on_quote = inner.view.iter_at_location(bx, by).is_some_and(|iter| {
            iter.tags()
                .iter()
                .any(|tag| tag.name().as_deref() == Some("quoted"))
        });
        if on_quote {
            inner.fold(!inner.quote_folded.get());
        }
    });
    inner.view.add_controller(click);
}

/// Where the fold hides the quote's text: everything after its first lines.
pub(super) fn hidden(text: &str) -> Option<u32> {
    let mut seen = 0;
    for (index, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == SHOWN_LINES {
                return Some(index as u32);
            }
        }
    }
    None
}

impl Inner {
    fn fold(&self, folded: bool) {
        self.quote_folded.set(folded);
        let quotes: Vec<std::ops::Range<u32>> = self
            .projection
            .borrow()
            .runs()
            .filter(|(_, run)| {
                matches!(
                    run,
                    Run::Atomic {
                        kind: AtomicKind::Quoted,
                        ..
                    }
                )
            })
            .map(|(at, run)| at..at + run.len())
            .collect();
        for range in quotes {
            super::render::retag(self, range);
        }
    }
}

impl NativeSurface {
    pub fn quote_folded(&self) -> bool {
        self.inner.quote_folded.get()
    }

    pub fn set_quote_folded(&self, folded: bool) {
        self.inner.fold(folded);
    }
}
