//! Selection, copy and links over the snapshot (spec 006 FR-017, FR-019).

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_gtk::body_view::BodyView;

use crate::body_view::{content, until};

fn shown(name: &str) -> (gtk::Window, gtk::ScrolledWindow, BodyView) {
    let view = BodyView::new(crate::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content(name));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    (window, scroller, view)
}

fn clipboard_text(clipboard: &gdk::Clipboard) -> Option<String> {
    let text = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = text.clone();
    clipboard.read_text_async(None::<&gtk::gio::Cancellable>, move |result| {
        *slot.borrow_mut() = Some(result.ok().flatten().map(|s| s.to_string()));
    });
    until(|| text.borrow().is_some());
    text.borrow().clone().flatten()
}

pub fn a_drag_across_cells_selects_and_copies_them_as_rows() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, _scroller, view) = shown("html-transactional-receipt");
    let doc = view.document().expect("a snapshot");
    let from = doc.text.rects(doc.text.find("The Salt Road")[0].clone())[0];
    let to = doc.text.rects(doc.text.find("38.00")[0].clone())[0];
    view.drag_select(
        gtk::graphene::Point::new((from.x0 + 1.0) as f32, from.center().y as f32),
        gtk::graphene::Point::new((to.x1 - 1.0) as f32, to.center().y as f32),
    );
    let range = view.selection().expect("the drag selected something");
    let text = doc.text.slice(range.clone()).to_owned();
    assert!(text.starts_with("The Salt Road"), "{text:?}");
    assert!(
        text.contains("\t1\t14.00\nField Guide to Lichens\t2\t38.00"),
        "{text:?}"
    );
    assert!(!view.selection_rects().is_empty(), "no highlight is drawn");

    let display = gdk::Display::default().expect("a display");
    assert_eq!(
        clipboard_text(&display.primary_clipboard()).as_deref(),
        Some(text.as_str()),
        "the primary clipboard holds the selection"
    );
    view.activate_action("clipboard.copy", None)
        .expect("a copy action");
    assert_eq!(
        clipboard_text(&display.clipboard()).as_deref(),
        Some(text.as_str())
    );
    window.destroy();
}

pub fn double_and_triple_clicks_select_a_word_and_a_line() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, _scroller, view) = shown("html-transactional-receipt");
    let doc = view.document().expect("a snapshot");
    let lichens = doc.text.find("Lichens")[0].clone();
    let at = doc.text.rects(lichens.clone())[0].center();
    let point = gtk::graphene::Point::new(at.x as f32, at.y as f32);
    view.click_select(point, 2);
    assert_eq!(view.selection(), Some(lichens.clone()));
    view.click_select(point, 3);
    let line = doc.text.line_at(lichens.start);
    assert_eq!(view.selection(), Some(line));
    let _ = glib::MainContext::default();
    window.destroy();
}
