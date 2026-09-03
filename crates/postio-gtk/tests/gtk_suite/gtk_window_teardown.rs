//! A destroyed window lets go of everything it built.
//!
//! #794. A test binary that stands up a `WebView` passed and then died on
//! the way out, WebKit reporting once per live view that the WebProcess had
//! not exited after the UI process closed the connection.
//!
//! Three `Window -> … -> Window` cycles kept every window ever built alive,
//! and with it its `Reader`, its `WebContext` — which *is* a WebProcess —
//! and its `NetworkSession`:
//!
//!   * `reader.connect_rendered` and `reader.connect_command` store handlers
//!     in the `Reader`, which the window's imp stores in turn;
//!   * the blob `source` closure becomes the `Rc<dyn BlobSource>` the reader
//!     hands to its `WebContext`, so that one closes the loop *inside
//!     WebKit* — which is why destroying the window never broke it.
//!
//! **Any one of the three kept the window alive.** Each was fixed alone
//! first and looked like no fix at all; that is the thing to remember if a
//! fourth appears.
//!
//! Asserted as a leak rather than as a crash, deliberately. The segfault is
//! a race that has never reproduced on this workstation — the binary that
//! failed on CI passes 25 runs of 25 here — so reproducing it was never
//! going to be the test. The leak underneath it takes milliseconds and is
//! exact.

use gtk::prelude::*;
use postio_gtk::window::Window;

use crate::settle;

pub fn a_destroyed_window_releases_its_reader_and_its_web_process() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let (window, view) = {
        let window = Window::default();
        // Build the reader: it is what owns the WebContext, and the point.
        let view = window.reader().view().downgrade();
        let weak = window.downgrade();
        window.destroy();
        (weak, view)
    };
    settle();

    assert!(
        window.upgrade().is_none(),
        "the window outlived its own destruction, so something still holds \
         it — one of the three cycles is back, or there is a fourth"
    );
    assert!(
        view.upgrade().is_none(),
        "the reader's WebView outlived the window that built it; its \
         WebContext is a WebProcess, and it is still attached at exit(), \
         which is #794"
    );
}

/// Destroying is required, and that part is GTK's, not ours.
///
/// A `GtkWindow` joins the toplevel list when it is *constructed*, not when
/// it is presented, and leaves on destroy. So dropping the Rust handle is
/// never enough on its own — which is worth asserting, because it is the
/// half a reader of the fix above would otherwise assume was unnecessary.
pub fn dropping_a_window_without_destroying_it_is_not_enough() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let window = {
        let window = Window::default();
        window.downgrade()
    };
    settle();

    assert!(
        window.upgrade().is_some(),
        "if a dropped window is now released on its own, GTK's toplevel \
         behaviour changed and the teardown this suite does can be dropped"
    );
    // Leave nothing behind for the next case.
    if let Some(window) = window.upgrade() {
        window.destroy();
    }
    settle();
}
