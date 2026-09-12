//! The application skeleton, on a real display: a window that opens, wears the
//! generated tokens, follows the system colour scheme and reports how long it
//! took to get there.
//!
//! GTK is single-threaded and single-init, so — as in `gtk_style.rs` — each
//! case here runs a whole sequence in order rather than being split into one
//! assertion apiece. Without a display they skip and say so; run them in a
//! session, or under a compositor, for the real thing. Nothing here touches
//! the network.

use crate::pump;
use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio};
use postio_gtk::startup::{Phase, Timeline};
use postio_gtk::{app, fonts, style, window::Window};

pub fn the_window_opens_and_wears_the_design() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();

    // Fonts before the first widget: a PangoContext keeps the family it has
    // already resolved. This is the order `app::run` uses too.
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the application object ────────────────────────────────────────────
    let application = app::build();
    assert_eq!(application.application_id().as_deref(), Some(app::APP_ID));

    // ── the icon the shell will draw ──────────────────────────────────────
    let theme = gtk::IconTheme::for_display(&display);
    assert!(
        theme.has_icon(app::APP_ID),
        "the bundled icon should resolve by application ID; \
         search path is {:?}",
        theme.resource_path()
    );

    // ── the window ────────────────────────────────────────────────────────
    let timeline = Timeline::start();
    timeline.mark(Phase::Init);
    let window = Window::default();
    timeline.mark(Phase::Window);

    assert_eq!(window.title().as_deref(), Some("Postio"));
    let (width, height) = window.default_size();
    assert!(
        width >= 1120 && height >= 640,
        "the default size should open on the canvas' three-pane proportions, \
         got {width}x{height}"
    );

    // A real AdwHeaderBar, so this reads as a GNOME application rather than a
    // GTK canvas in a bare frame.
    assert!(
        find_header_bar(window.upcast_ref::<gtk::Widget>()).is_some(),
        "the window should carry an AdwHeaderBar"
    );

    // ── it reaches the screen, and we know when ───────────────────────────
    let frames = Rc::new(Cell::new(0u32));
    postio_gtk::startup::on_first_frame(&window, {
        let timeline = timeline.clone();
        let frames = frames.clone();
        move || {
            timeline.mark(Phase::FirstFrame);
            frames.set(frames.get() + 1);
        }
    });

    window.present();
    pump();

    assert!(window.is_mapped(), "the window should be on screen");
    assert_eq!(
        frames.get(),
        1,
        "the first-frame hook should fire exactly once"
    );
    assert!(
        timeline.total().is_some(),
        "startup should be instrumented through to the first frame"
    );
    let report = timeline.report();
    assert!(report.starts_with("startup "), "{report}");

    // ── the generated tokens reach it, and follow the system scheme ───────
    let manager = adw::StyleManager::default();

    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    pump();
    assert!(!window.has_css_class(style::DARK_CLASS));
    let light = window.color();

    manager.set_color_scheme(adw::ColorScheme::ForceDark);
    pump();
    assert!(
        window.has_css_class(style::DARK_CLASS),
        "the window should pick up the dark class when the system does"
    );
    let dark = window.color();

    assert_ne!(
        (light.red(), light.green(), light.blue()),
        (dark.red(), dark.green(), dark.blue()),
        "the tokens should actually repaint the window, not just tag it"
    );
    assert!(
        luma(dark) > luma(light),
        "dark ink should be light-on-dark: {dark:?} vs {light:?}"
    );

    manager.set_color_scheme(adw::ColorScheme::Default);
    window.destroy();
}

/// The window the application opens is the one its startup is recorded in.
///
/// `app::build_with` marks [`Phase::Window`] itself, and hands the same
/// timeline to the window so that whoever points the panes at the store can
/// mark the phases in between — `postio_app::feed_the_window` holds a
/// `&Window` and nothing else a timeline could arrive through (#1479).
///
/// One line in `build_with` does that, and a line nothing exercises is this
/// project's characteristic bug: the marks would simply never be recorded,
/// and the failure is silent — a report with two phases missing reads as a
/// startup that skipped them rather than as an instrument that was never
/// connected.
pub fn the_application_hands_its_window_the_startup_timeline() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let timeline = Timeline::start();
    let application = app::build_with(timeline.clone());
    // **Non-unique, and this is not a detail.** `build_with` builds an
    // application under the real `APP_ID`, so registering it on a machine
    // where Postio is actually running would make this the *remote* instance
    // and `activate()` would raise the maintainer's own window instead of
    // opening one here. A test must not be able to do that.
    application.set_flags(gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gio::Cancellable>)
        .expect("the application should register");
    application.activate();
    pump();

    let window = application
        .active_window()
        .and_downcast::<Window>()
        .expect("activate should open a Postio window");
    assert!(
        timeline.at(Phase::Window).is_some(),
        "build_with marks the window phase itself"
    );

    // The phases `postio-app` marks, from the only handle it has.
    assert_eq!(timeline.at(Phase::Feeds), None, "nothing has marked it yet");
    window.mark_startup(Phase::Feeds);
    assert!(
        timeline.at(Phase::Feeds).is_some(),
        "the window the application opened is not carrying the timeline its \
         own startup is being recorded in, so every phase between the window \
         and the first frame would go unmarked and the trace would say the \
         first frame was all of it"
    );

    window.destroy();
    pump();
}

/// Depth-first walk for the header bar, so the test does not have to know how
/// the toolbar view nests it.
fn find_header_bar(widget: &gtk::Widget) -> Option<adw::HeaderBar> {
    if let Ok(bar) = widget.clone().downcast::<adw::HeaderBar>() {
        return Some(bar);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_header_bar(&current) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn luma(c: gdk::RGBA) -> f32 {
    0.2126 * c.red() + 0.7152 * c.green() + 0.0722 * c.blue()
}
