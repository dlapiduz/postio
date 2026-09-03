//! Rendering a window to a PNG, and saying so when it cannot.
//!
//! `/gtk-design` makes rendering a screen and looking at it the last step
//! before a surface is called done. #809 is what happens when that step
//! stops working: `shot` printed one line about a missing frame, wrote no
//! file, and the session that ran it had nothing to look at — while a
//! session that did not check for the file would have reported "rendered and
//! checked" in good faith.
//!
//! Three properties, and the middle one is the whole issue:
//!
//!   * a window the compositor never showed cannot be captured, and that is
//!     an error rather than an empty picture;
//!   * a failed capture leaves **no file** — so the file's existence is a
//!     fact a caller can rely on, and `shot` can exit non-zero;
//!   * a presented window is captured without the caller counting frames,
//!     and is not reported as coming off a stalled compositor. The copies of
//!     this logic that #809 found each made the caller settle first; the
//!     wait belongs to the thing that knows what it is waiting for.
//!
//! Skips without a display. Nothing here touches the network.

use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::capture;

/// Long enough that a loaded machine is not mistaken for a broken one, short
/// enough that the two negative cases do not dominate the suite. Scaled by
/// `POSTIO_TEST_PATIENCE` inside `capture`, like every other deadline here.
const BRIEF: Duration = Duration::from_millis(500);

/// A window with something in it worth drawing.
fn window() -> gtk::Window {
    let window = gtk::Window::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let label = gtk::Label::new(Some("postio"));
    label.set_hexpand(true);
    label.set_vexpand(true);
    content.append(&label);
    window.set_child(Some(&content));
    window.set_default_size(400, 300);
    window
}

pub fn a_window_the_compositor_never_showed_is_an_error() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let window = window();
    // Realized but never presented: the widgets exist and are laid out on
    // demand, and GTK still refuses to snapshot an unmapped widget. That is
    // the floor #809 was looking for and did not find — there is no path
    // that renders a widget tree with no compositor at all.
    gtk::prelude::WidgetExt::realize(&window);

    let outcome = capture::texture_within(&window, BRIEF);
    assert!(
        outcome.is_err(),
        "a window that was never presented reported a capture: {outcome:?}"
    );
}

pub fn a_capture_that_fails_leaves_no_file() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let directory = tempfile::tempdir().expect("a directory to not write into");
    let path = directory.path().join("never-written.png");

    let window = window();
    gtk::prelude::WidgetExt::realize(&window);
    let outcome = capture::png_within(&window, &path, BRIEF);

    assert!(outcome.is_err(), "a failed capture reported success");
    assert!(
        !path.exists(),
        "a failed capture left {} behind, so its existence proves nothing",
        path.display()
    );
}

pub fn a_presented_window_is_captured_without_the_caller_counting_frames() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let window = window();
    window.present();
    // Deliberately no settle, no pump, no frame count. Every copy of this
    // logic #809 found asked its caller to do that first, which is how one
    // of them came to render an empty message list (#596) and how another
    // gave up after eight frames that never came.
    let picture = match capture::texture_within(&window, Duration::from_secs(30)) {
        Ok(picture) => picture,
        Err(error) => panic!("a presented window would not render: {error}"),
    };

    assert_eq!(
        (picture.texture.width(), picture.texture.height()),
        (window.width(), window.height()),
        "the capture is not the size the window was allocated"
    );
    // The suites run on a compositor that presents, so this is the value
    // that says the stalled-surface warning stays quiet when nothing is
    // wrong. It is the caveat `shot` prints, and a caveat printed on every
    // shot is a caveat nobody reads.
    assert!(
        !picture.stalled,
        "a window on a presenting compositor was reported as stalled"
    );
    window.destroy();
}
