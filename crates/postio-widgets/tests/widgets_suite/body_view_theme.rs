//! The reading surface follows the app's theme (spec 006 FR-011): one
//! re-render per change, the reading position kept.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_render::Presentation;
use postio_widgets::body_view::BodyView;

use crate::body_view::{content, until};

pub fn a_theme_change_re_renders_once_and_keeps_the_place() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceLight);
    let view = BodyView::new(crate::support_reader::reader_deadline());
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
