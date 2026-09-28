//! The classic single-message reader folds a quote behind the line that
//! says how much it hides, and opens it on a click (specs/007-postio-focus
//! T067, FR-034): the same folds Focus's dialog shows, from the one reader.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;

use crate::pump;

fn reply() -> MessageBody {
    MessageBody {
        text: Some(
            "Fine by me.\n\n> Short note before the walkthrough tomorrow.\n> Three things, in order.\n> The layout first.\n\nQuinn\n"
                .to_owned(),
        ),
        html: None,
    }
}

pub fn a_reply_s_quote_is_folded_and_opens_when_clicked() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = Window::default();
    gtk::prelude::GtkWindowExt::present(&window);
    pump();

    window.show_message(&reply(), Some("ada@example.com"));
    let view = window.reader().view().clone();
    crate::settle_until("the reply to render its fold", || !view.folds().is_empty());
    assert_eq!(
        view.folds(),
        [("q0".to_owned(), "3 quoted lines".to_owned(), false)],
        "the quote is folded behind its line count"
    );
    let quoted = |view: &postio_gtk::body_view::BodyView| {
        view.document()
            .is_some_and(|d| !d.text.find("walkthrough tomorrow").is_empty())
    };
    assert!(!quoted(&view), "the quoted words are hidden while folded");

    let document = view.document().expect("rendered");
    let fold = document.folds.first().expect("a fold");
    let top = gtk::prelude::ScrollableExt::vadjustment(&view)
        .map_or(0.0, |a| gtk::prelude::AdjustmentExt::value(&a));
    view.click_select(
        gtk::graphene::Point::new(
            ((fold.summary_rect.x0 + fold.summary_rect.x1) / 2.0) as f32,
            ((fold.summary_rect.y0 + fold.summary_rect.y1) / 2.0 - top) as f32,
        ),
        1,
    );
    crate::settle_until("the fold to open", || {
        view.folds().first().is_some_and(|(_, _, open)| *open) && quoted(&view)
    });
    gtk::prelude::GtkWindowExt::destroy(&window);
}
