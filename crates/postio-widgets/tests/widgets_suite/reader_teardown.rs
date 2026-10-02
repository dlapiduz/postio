//! A dropped `Reader` really lets go of its body view.
//!
//! #794 began as WebKit's: a test binary that stood up a reader passed and
//! then died on the way out, because every `Reader` built a `WebContext` --
//! a WebProcess -- that a dropped reader never released. The reader's body
//! is a `BodyView` now (spec 006), and the same leak costs a render thread,
//! a snapshot and its tiles per reader instead of a process; the assertion
//! is the same one, on the mechanism, because it is deterministic.
//!
//! The switch reintroduced it once: the view's own signal handlers held the
//! reader's `Place`, and `Place` held the view.
//!
//! So: hold a weak reference, drop the reader, turn the loop, and require
//! the view to be gone.

use std::rc::Rc;

use glib::object::ObjectExt;
use postio_widgets::reader::Reader;

use crate::support_reader::settle;

pub fn a_dropped_reader_releases_its_view() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    // A weak reference, so holding it cannot be what keeps the view alive.
    let weak = {
        let reader = Reader::new(Rc::new(|_content_id: &str| None));
        let weak = reader.view().downgrade();
        assert!(
            weak.upgrade().is_some(),
            "the view should be alive while the reader is"
        );
        weak
    };

    // GTK finalizes on the main loop, not at the closing brace.
    settle();

    assert!(
        weak.upgrade().is_none(),
        "the reader was dropped and its body view is still alive, and with \
         it a render thread, a snapshot and its tiles: every reader a \
         process builds is kept for its lifetime (#794)"
    );
}

/// The same thing several times over, because one leak and a hundred leaks
/// fail this the same way but are very different in memory.
pub fn readers_do_not_accumulate_views() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let mut weaks = Vec::new();
    for _ in 0..5 {
        let reader = Reader::new(Rc::new(|_content_id: &str| None));
        weaks.push(reader.view().downgrade());
    }
    settle();

    let alive = weaks.iter().filter(|w| w.upgrade().is_some()).count();
    assert_eq!(
        alive, 0,
        "{alive} of 5 body views outlived the readers that made them; each \
         one is a render thread and a snapshot kept for the process's life"
    );
}
