//! Quote folds in a single message (specs/007-postio-focus T067, FR-034):
//! the quoted part is folded behind a line that says how much it hides,
//! and opens when that line is activated -- clicked, or asked for by the
//! dialog's own fold line through `open_fold`.

use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::body_view::BodyView;

use crate::support::{content, reader_deadline, until};

/// Something only the quoted part says.
const QUOTED: &str = "walkthrough tomorrow";

fn shown(name: &str) -> (BodyView, gtk::Window) {
    let view = BodyView::new(reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content_from_top(content(name));
    assert!(
        until(|| view.document().is_some_and(|d| !d.folds.is_empty())),
        "the reply rendered no fold"
    );
    (view, window)
}

fn quoted_shown(view: &BodyView) -> bool {
    view.document()
        .is_some_and(|d| d.folds.first().is_some_and(|f| f.open) && !d.text.find(QUOTED).is_empty())
}

pub fn a_quote_is_folded_behind_its_line_count_and_opens_when_clicked() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (view, window) = shown("plain-text-flowed-reply");
    let document = view.document().expect("rendered");
    let fold = document.folds.first().expect("a fold").clone();
    assert_eq!(fold.id, "q0");
    assert!(!fold.open, "the quote starts folded");
    assert!(
        !document.text.find("2 quoted lines").is_empty(),
        "the fold says how much it hides"
    );
    assert!(
        !quoted_shown(&view),
        "the quote's words are hidden while folded"
    );

    let top = view.vadjustment().map_or(0.0, |a| a.value());
    let at = gtk::graphene::Point::new(
        ((fold.summary_rect.x0 + fold.summary_rect.x1) / 2.0) as f32,
        ((fold.summary_rect.y0 + fold.summary_rect.y1) / 2.0 - top) as f32,
    );
    view.click_select(at, 1);
    assert!(
        until(|| quoted_shown(&view)),
        "clicking the fold did not open it"
    );
    window.destroy();
}

pub fn the_dialog_s_fold_line_opens_the_quote_it_names() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (view, window) = shown("plain-text-flowed-reply");
    assert_eq!(
        view.folds(),
        [("q0".to_owned(), "2 quoted lines".to_owned(), false)],
        "the folds a surface outside the body can name"
    );
    view.open_fold("q0");
    assert!(until(|| quoted_shown(&view)), "open_fold did not open it");
    view.open_fold("q0");
    assert!(
        until(|| quoted_shown(&view)),
        "opening an open fold closed it: open_fold opens, it does not toggle"
    );
    window.destroy();
}
