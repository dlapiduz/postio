//! The screen reader reads what copy copies (spec 006 FR-020): the view
//! is a document whose accessible text is the snapshot's text index.

use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::body_view::BodyView;

use crate::body_view::{content, until};

pub fn the_accessible_text_is_the_text_index() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(crate::support_reader::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content("html-transactional-receipt"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let doc = view.document().expect("a snapshot");

    assert_eq!(view.accessible_role(), gtk::AccessibleRole::Document);
    assert_eq!(
        view.accessible_contents(0, u32::MAX).as_deref(),
        Some(doc.text.text.as_str())
    );

    // The caret and the selection are the view's selection.
    let range = doc.text.find("Gift wrap")[0].clone();
    let rect = doc.text.rects(range.clone())[0];
    view.drag_select(
        gtk::graphene::Point::new((rect.x0 + 1.0) as f32, rect.center().y as f32),
        gtk::graphene::Point::new((rect.x1 - 1.0) as f32, rect.center().y as f32),
    );
    assert_eq!(view.accessible_selection(), vec![range.clone()]);
    assert_eq!(view.accessible_caret(), range.end as u32);

    // Extents are the clusters' union, where the view draws them.
    let extents = view
        .accessible_extents(range.start as u32, range.end as u32)
        .expect("extents");
    assert!((f64::from(extents.x()) - rect.x0).abs() < 1.0);
    assert!((f64::from(extents.width()) - rect.width()).abs() < 1.0);
    window.destroy();
}
