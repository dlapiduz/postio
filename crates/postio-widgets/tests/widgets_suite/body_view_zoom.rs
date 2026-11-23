use gtk::prelude::*;
use postio_widgets::body_view::{BodyView, zoom::ZoomIndicator};

use crate::body_view::{content, until};

pub fn zoom_steps_keep_the_place_and_the_selection() {
    let view = BodyView::new(crate::support_reader::reader_deadline());
    let indicator = ZoomIndicator::new(&view);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .build();
    let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pane.append(indicator.widget());
    pane.append(&scroller);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&pane)
        .build();
    window.present();
    view.set_content(content("html-very-tall"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    assert_eq!(view.zoom(), 100);
    assert!(
        !indicator.widget().is_visible(),
        "the indicator shows at 100%"
    );

    let adjustment = scroller.vadjustment();
    adjustment.set_value(3000.0);
    let doc = view.document().expect("a snapshot");
    let top = doc.text.char_at_top(3000.0);
    let range = doc.text.find("Line 70 of")[0].clone();
    let rect = doc.text.rects(range.clone())[0];
    let origin = adjustment.value();
    view.drag_select(
        gtk::graphene::Point::new((rect.x0 + 1.0) as f32, (rect.center().y - origin) as f32),
        gtk::graphene::Point::new((rect.x1 - 1.0) as f32, (rect.center().y - origin) as f32),
    );
    let selected = view.selection();

    for expected in [110, 125] {
        let generation = view.document().expect("a snapshot").generation;
        view.zoom_in();
        assert_eq!(view.zoom(), expected);
        assert!(until(|| view
            .document()
            .is_some_and(|d| d.generation > generation)));
        let doc = view.document().expect("the zoomed snapshot");
        let kept = doc.text.char_at_top(adjustment.value());
        assert!(
            kept.abs_diff(top) <= 40,
            "at {expected}% the top moved from {top} to {kept}"
        );
    }
    assert!(
        indicator.widget().is_visible(),
        "the indicator is hidden at 125%"
    );
    assert_eq!(indicator.label(), "125%");
    assert_eq!(view.selection(), selected, "zooming lost the selection");

    let doc = view.document().expect("a snapshot");
    let pointer = gtk::graphene::Point::new(100.0, 200.0);
    let under = doc
        .text
        .hit(postio_render::Point::new(100.0, adjustment.value() + 200.0))
        .unwrap_or(0);
    let generation = doc.generation;
    view.scroll_zoom(-1.0, pointer);
    assert_eq!(view.zoom(), 150);
    assert!(until(|| view
        .document()
        .is_some_and(|d| d.generation > generation)));
    let doc = view.document().expect("the zoomed snapshot");
    let now = doc
        .text
        .hit(postio_render::Point::new(100.0, adjustment.value() + 200.0))
        .unwrap_or(0);
    assert!(
        now.abs_diff(under) <= 40,
        "the text under the pointer moved: {under} -> {now}"
    );

    for _ in 0..10 {
        view.zoom_in();
    }
    assert_eq!(view.zoom(), 300);
    view.zoom_in();
    assert_eq!(view.zoom(), 300);

    indicator.reset();
    assert_eq!(view.zoom(), 100);
    assert!(!indicator.widget().is_visible());
    window.destroy();
}

pub fn a_pinch_snaps_to_a_step_and_renders_once() {
    let view = BodyView::new(crate::support_reader::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content("html-newsletter"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let (before, _) = postio_ui::test_support::snapshot_counts();
    for scale in [1.1, 1.2, 1.3] {
        view.pinch_update(scale);
        for _ in 0..5 {
            gtk::glib::MainContext::default().iteration(false);
        }
    }
    assert!(view.pinching(), "the pinch is not drawn while it runs");
    let (during, _) = postio_ui::test_support::snapshot_counts();
    assert_eq!(during, before, "a render was issued mid-pinch");
    view.pinch_end();
    assert_eq!(view.zoom(), 125, "130% is nearest 125%");
    assert!(!view.pinching());
    assert!(until(
        || postio_ui::test_support::snapshot_counts().0 == before + 1
    ));
    for _ in 0..20 {
        gtk::glib::MainContext::default().iteration(false);
    }
    assert_eq!(
        postio_ui::test_support::snapshot_counts().0,
        before + 1,
        "more than one render"
    );
    window.destroy();
}
