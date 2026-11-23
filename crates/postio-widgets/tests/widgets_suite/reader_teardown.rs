use std::rc::Rc;

use glib::object::ObjectExt;
use postio_widgets::reader::Reader;

use crate::support_reader::settle;

pub fn a_dropped_reader_releases_its_view() {
    let weak = {
        let reader = Reader::new(Rc::new(|_content_id: &str| None));
        let weak = reader.view().downgrade();
        assert!(
            weak.upgrade().is_some(),
            "the view should be alive while the reader is"
        );
        weak
    };

    settle();

    assert!(
        weak.upgrade().is_none(),
        "the reader was dropped and its body view is still alive, and with \
         it a render thread, a snapshot and its tiles: every reader a \
         process builds is kept for its lifetime (#794)"
    );
}

pub fn readers_do_not_accumulate_views() {
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
