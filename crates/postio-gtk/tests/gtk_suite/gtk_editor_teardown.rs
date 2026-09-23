//! Closing a composer must release its WebView and WebKit process.

use std::rc::Rc;

use glib::object::ObjectExt;
use postio_gtk::editor;
use postio_gtk::reader::scheme::BlobSource;

use crate::settle;

struct NoBlobs;

impl BlobSource for NoBlobs {
    fn resolve(&self, _content_id: &str) -> Option<(Vec<u8>, String)> {
        None
    }
}

pub fn closing_editors_releases_their_webviews() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let mut weaks = Vec::new();
    for _ in 0..3 {
        let view = editor::editing_view(Rc::new(NoBlobs));
        weaks.push(view.downgrade());
        let editor = editor::Editor::new(Rc::new(NoBlobs));
        weaks.push(editor.widget().downgrade());
    }
    settle();

    let alive = weaks.iter().filter(|weak| weak.upgrade().is_some()).count();
    assert_eq!(
        alive, 0,
        "{alive} editor WebViews outlived their composers; each can retain a WebKit process"
    );
}
