//! A dropped `Reader` really lets go of its `WebView`.
//!
//! #794: a test binary that stands up a reader passes and then dies on the
//! way out, with WebKit saying — once per live view —
//!
//!     WebProcess didn't exit as expected after the UI process connection
//!     was closed
//!
//! and the UI process then taking SIGSEGV. It is intermittent, about the
//! rate #699 ran at, so reproducing the crash is a poor way to test for it:
//! a green run proves almost nothing.
//!
//! This asserts the *mechanism* instead, which is deterministic. Every
//! `Reader` builds its own `WebContext` and ephemeral `NetworkSession`, and
//! a `WebContext` is a WebProcess. If a dropped `Reader` does not release
//! its view, those processes accumulate for the life of the binary and are
//! still attached when `exit()` tears the connection down underneath them —
//! which is exactly what WebKit is complaining about.
//!
//! So: hold a weak reference, drop the reader, turn the loop, and require
//! the view to be gone. A leak fails here in milliseconds and says which
//! object survived, rather than failing one run in eight somewhere else with
//! a signal number.

use std::rc::Rc;

use glib::object::ObjectExt;
use postio_gtk::reader::Reader;

use crate::settle;

pub fn a_dropped_reader_releases_its_webview() {
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
        "the reader was dropped and its WebView is still alive, so its \
         WebContext -- a WebProcess -- is too. Every reader a binary builds \
         then survives to `exit()`, where WebKit reports `WebProcess didn't \
         exit as expected after the UI process connection was closed` and \
         the process segfaults (#794)."
    );
}

/// The same thing several times over, because one leak and a hundred leaks
/// fail this the same way but are very different at teardown — and the
/// reported crash showed *three* WebProcesses complaining, not one.
pub fn readers_do_not_accumulate_webviews() {
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
        "{alive} of 5 WebViews outlived the readers that made them; each one \
         is a WebProcess still attached at exit"
    );
}
