//! A highlighted range in the body (specs/007-postio-focus T065, research
//! R2): the sentence a marker quotes, found in the rendered text, is drawn
//! over its own rectangles and brought into view -- and belongs to the
//! message it was found in.

use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::body_view::BodyView;

use crate::support::{conversation, reader_deadline, until};

const TARGET: &str = "Please leave comments by Wednesday";

/// A message long enough that its last paragraph starts below the first
/// screen, ending in the sentence a marker would quote.
fn long_message() -> String {
    let mut html = String::new();
    for n in 0..60 {
        html.push_str(&format!(
            "<p>Paragraph {n} of the draft, about the pagination changes and the rate limits.</p>"
        ));
    }
    html.push_str(&format!(
        "<p>{TARGET}; I would like to freeze it Thursday.</p>"
    ));
    html
}

pub fn a_highlighted_range_is_drawn_over_its_rectangles_and_scrolled_into_view() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();

    view.set_content_from_top(conversation("", &long_message()));
    assert!(
        until(|| view
            .document()
            .is_some_and(|d| !d.text.find(TARGET).is_empty())),
        "the message never rendered"
    );
    let document = view.document().expect("rendered");
    let range = document.text.find(TARGET)[0].clone();
    let expected = document.text.rects(range.clone());
    assert!(!expected.is_empty(), "the sentence has rectangles");
    let page = scroller.vadjustment().page_size();
    assert!(
        expected[0].y0 > page,
        "the fixture's sentence starts on the first screen, so a scroll proves nothing"
    );

    view.set_highlight(Some(range.clone()));
    assert_eq!(
        view.highlight_rects(),
        expected,
        "the highlight covers exactly the range's rectangles"
    );
    assert!(
        until(|| {
            let adjustment = scroller.vadjustment();
            let (top, bottom) = (
                adjustment.value(),
                adjustment.value() + adjustment.page_size(),
            );
            expected
                .iter()
                .all(|rect| rect.y0 >= top && rect.y1 <= bottom)
        }),
        "the highlighted sentence was not brought into view"
    );

    view.set_content_from_top(conversation("", "<p>Another message.</p>"));
    assert!(
        view.highlight_rects().is_empty(),
        "a highlight belongs to its message, not the next"
    );
    window.destroy();
}
