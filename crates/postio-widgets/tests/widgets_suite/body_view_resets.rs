//! A new message starts as it was sent (specs/007-postio-focus research R1).
//!
//! `set_content_from_top` is how a reader shows a *different* message, not a
//! redraw of the one on screen. What the user did to the last one -- darkened
//! it, selected in it, walked to one of its links, opened one of its folds --
//! belongs to that message and must not be drawn over the next. Focus's
//! dialog reuses one view for every open, so there the next message is always
//! the one that would inherit it.

use gtk::gdk;
use gtk::prelude::*;
use postio_render::Presentation;
use postio_widgets::body_view::BodyView;

use crate::support::{content, conversation, reader_deadline, until};

fn window_with(view: &BodyView) -> gtk::Window {
    let scroller = gtk::ScrolledWindow::builder().child(view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    window
}

/// Spec 006 FR-013a: darkening "applies to the one message it was asked of
/// and is not remembered across openings". A single message's scope is the
/// empty string, so a darkened scope names every message shown after it.
pub fn a_message_shown_after_a_darkened_one_is_not_darkened() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceDark);
    let view = BodyView::new(reader_deadline());
    let window = window_with(&view);
    let presented = |want: Presentation| {
        until(|| {
            view.document()
                .is_some_and(|d| d.messages.first().is_some_and(|m| m.presentation == want))
        })
    };

    view.set_content_from_top(content("html-newsletter"));
    assert!(
        presented(Presentation::Paper),
        "the first message is not paper in dark"
    );
    assert!(
        view.toggle_darken(),
        "the first message could not be darkened"
    );
    assert!(
        presented(Presentation::Darkened),
        "darkening did not darken it"
    );

    view.set_content_from_top(content("html-designed-three-column"));
    assert!(
        presented(Presentation::Paper),
        "the next message was drawn {:?}: darkening the last one carried over to it",
        view.document()
            .and_then(|d| d.messages.first().map(|m| m.presentation))
    );
    assert_eq!(
        view.darken_title(),
        Some("Darken this message"),
        "the next message's darken command offers to undo a darkening it never had"
    );
    style.set_color_scheme(adw::ColorScheme::Default);
    window.destroy();
}

/// A selection and a focused link are places in one message's text: drawn
/// over another message, they highlight words and ring a link the user never
/// chose, and Return follows it.
pub fn a_message_shown_after_another_has_no_selection_and_no_focused_link() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(reader_deadline());
    let window = window_with(&view);
    let first = conversation(
        "7",
        "<p>Lichens grow slowly on <a href=\"https://example.com/rocks\">bare rock</a>.</p>",
    );
    view.set_content_from_top(first);
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let document = view.document().expect("a snapshot");
    let at = document
        .text
        .text
        .find("Lichens")
        .map(|byte| document.text.text[..byte].chars().count())
        .expect("the first message's text");
    let word = document.text.word_at(at);
    let rect = document.text.rects(word.clone())[0];
    view.click_select(
        gtk::graphene::Point::new(rect.center().x as f32, rect.center().y as f32),
        2,
    );
    assert_eq!(view.selection(), Some(word), "the word was not selected");
    view.focus_next_link(true);
    assert!(
        view.focused_link_target().is_some(),
        "no link took keyboard focus"
    );

    let rendered = std::rc::Rc::new(std::cell::Cell::new(false));
    view.connect_rendered({
        let rendered = rendered.clone();
        move |_| rendered.set(true)
    });
    view.set_content_from_top(conversation(
        "8",
        "<p>Mosses need water; see <a href=\"https://example.org/moss\">the notes</a>.</p>",
    ));
    assert!(until(|| rendered.get()), "the next message never arrived");
    assert_eq!(
        view.selection(),
        None,
        "the last message's selection is drawn over the next one"
    );
    assert!(
        view.selection_rects().is_empty(),
        "a highlight is drawn over the next message"
    );
    assert_eq!(
        view.focused_link_target(),
        None,
        "keyboard focus is on a link in the next message that nobody moved it to"
    );
    window.destroy();
}

/// A fold the user opened is opened in that showing of the message. Shown
/// again later, from the top, the message folds as it was sent.
pub fn a_message_shown_again_folds_as_it_was_sent() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(reader_deadline());
    let window = window_with(&view);
    let body = "<p>Agreed, Thursday.</p>\
                <blockquote><p>Can we meet on Thursday?</p><p>Or Friday.</p></blockquote>";
    let quote_open = |view: &BodyView| {
        view.document().and_then(|d| {
            d.folds
                .iter()
                .find(|fold| fold.id.contains("-q"))
                .map(|fold| (fold.id.clone(), fold.open, fold.summary_rect))
        })
    };

    view.set_content_from_top(conversation("7", body));
    assert!(
        until(|| quote_open(&view).is_some()),
        "the quote was not folded"
    );
    let (id, open, summary) = quote_open(&view).expect("the quote's fold");
    assert!(!open, "the quote does not start folded");
    view.click_select(
        gtk::graphene::Point::new(summary.center().x as f32, summary.center().y as f32),
        1,
    );
    assert!(
        until(|| quote_open(&view).is_some_and(|(_, open, _)| open)),
        "clicking the fold's summary did not open {id}"
    );

    view.set_content_from_top(conversation("8", "<p>Something else entirely.</p>"));
    assert!(
        until(|| view
            .document()
            .is_some_and(|d| d.folds.iter().all(|f| !f.id.contains("-q")))),
        "the other message never arrived"
    );
    view.set_content_from_top(conversation("7", body));
    assert!(
        until(|| quote_open(&view).is_some()),
        "the first message never came back"
    );
    let (_, open, _) = quote_open(&view).expect("the quote's fold");
    assert!(
        !open,
        "shown again from the top, the message kept a fold the user opened last time"
    );
    window.destroy();
}
