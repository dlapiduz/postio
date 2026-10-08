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
use postio_widgets::capture;

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

/// A popover is a surface of its own, so a window's widgets drawn alone
/// leave it out -- and Focus's folders popover (screen 10) is exactly what
/// a picture of that screen is for. The capture draws an open popover
/// where the compositor shows it.
pub fn an_open_popover_is_in_the_picture() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let window = gtk::Window::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let anchor = gtk::Button::with_label("places");
    anchor.set_halign(gtk::Align::Start);
    anchor.set_valign(gtk::Align::Start);
    content.append(&anchor);
    let filler = gtk::Label::new(Some("postio"));
    filler.set_vexpand(true);
    content.append(&filler);
    window.set_child(Some(&content));
    window.set_default_size(400, 300);

    let red = gtk::DrawingArea::new();
    red.set_content_width(120);
    red.set_content_height(80);
    red.set_draw_func(|_, cairo, width, height| {
        cairo.set_source_rgb(1.0, 0.0, 0.0);
        cairo.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = cairo.fill();
    });
    let popover = gtk::Popover::builder().child(&red).has_arrow(false).build();
    popover.set_parent(&anchor);
    window.present();
    popover.popup();
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(Duration::from_millis(10), || {
        gtk::glib::ControlFlow::Continue
    });
    for _ in 0..3000 {
        if red.is_mapped() && red.width() > 0 {
            break;
        }
        context.iteration(true);
    }
    heartbeat.remove();
    assert!(red.is_mapped(), "the popover never opened");

    let picture = match capture::texture_within(&window, Duration::from_secs(30)) {
        Ok(picture) => picture,
        Err(error) => panic!("a presented window would not render: {error}"),
    };
    let texture = picture.texture;
    let (width, height) = (texture.width(), texture.height());
    let stride = usize::try_from(width).expect("a width") * 4;
    let mut pixels = vec![0u8; stride * usize::try_from(height).expect("a height")];
    texture.download(&mut pixels, stride);
    // Cairo's memory format is premultiplied BGRA, little end first.
    let red_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[2] > 240 && pixel[1] < 16 && pixel[0] < 16)
        .count();
    assert!(
        red_pixels > 120 * 80 / 2,
        "the popover's content is not in the picture: {red_pixels} red pixels"
    );
    popover.popdown();
    popover.unparent();
    window.destroy();
}
