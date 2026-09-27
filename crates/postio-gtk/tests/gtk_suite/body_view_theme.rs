//! The reading surface follows the app's theme (spec 006 FR-011): one
//! re-render per change, the reading position kept.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::body_view::BodyView;
use postio_render::Presentation;

use crate::body_view::{content, until};

pub fn a_theme_change_re_renders_once_and_keeps_the_place() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceLight);
    let view = BodyView::new(crate::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    // Tall enough to scroll, so there is a place to keep.
    view.set_content(content("html-very-tall"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let light = view.document().expect("a snapshot");
    assert_eq!(light.messages[0].presentation, Presentation::Styled);
    scroller.vadjustment().set_value(3000.0);
    assert_eq!(scroller.vadjustment().value(), 3000.0);
    let anchor = light.text.char_at_top(3000.0);

    let (before, _) = postio_ui::test_support::snapshot_counts();
    style.set_color_scheme(adw::ColorScheme::ForceDark);
    assert!(
        until(|| view
            .document()
            .is_some_and(|d| d.messages[0].presentation == Presentation::Adapted)),
        "going dark did not re-render the message for the dark reader"
    );
    for _ in 0..20 {
        glib::MainContext::default().iteration(false);
    }
    let (after, _) = postio_ui::test_support::snapshot_counts();
    assert_eq!(after - before, 1, "one theme change, one render");
    let dark = view.document().expect("the dark snapshot");
    let top = scroller.vadjustment().value();
    let kept = dark.text.char_at_top(top);
    assert!(
        kept.abs_diff(anchor) <= 8,
        "the place moved: {anchor} was at the top, now {kept}"
    );
    style.set_color_scheme(adw::ColorScheme::Default);
    window.destroy();
}

/// FR-013a: in dark mode a sheet of paper can be darkened, and the command
/// is its own undo -- its title says which way it goes.
pub fn darken_is_its_own_undo() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceDark);
    let view = BodyView::new(crate::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content("html-newsletter"));
    let presented = |want: Presentation| {
        until(|| {
            view.document()
                .is_some_and(|d| d.messages[0].presentation == want)
        })
    };
    assert!(
        presented(Presentation::Paper),
        "the newsletter is not paper in dark"
    );
    assert_eq!(view.darken_title().as_deref(), Some("Darken this message"));
    assert!(view.toggle_darken(), "paper could not be darkened");
    assert!(
        presented(Presentation::Darkened),
        "darkening did not re-render it darkened"
    );
    assert_eq!(view.darken_title().as_deref(), Some("Show as sent"));
    assert!(view.toggle_darken());
    assert!(
        presented(Presentation::Paper),
        "showing it as sent did not restore the paper"
    );
    assert_eq!(view.darken_title().as_deref(), Some("Darken this message"));
    style.set_color_scheme(adw::ColorScheme::Default);
    window.destroy();
}
