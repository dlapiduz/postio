//! Find in the message on screen (spec 006 FR-018).

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::body_view::{BodyView, find::FindBar};

use crate::body_view::{content, until};

pub fn find_highlights_steps_and_survives_a_re_render() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    // Light to start, so going dark later is a change.
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
    let view = BodyView::new(crate::reader_deadline());
    let bar = FindBar::new(&view);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .build();
    let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pane.append(bar.widget());
    pane.append(&scroller);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(300)
        .child(&pane)
        .build();
    window.present();
    view.set_content(content("html-very-tall"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let doc = view.document().expect("a snapshot");

    bar.open();
    // Focus within the window, which a headless compositor may never make
    // active; a search entry puts it on its own text child.
    let entry: gtk::Widget = bar.entry().clone().upcast();
    assert!(
        until(|| gtk::prelude::GtkWindowExt::focus(&window)
            .is_some_and(|focus| focus == entry || focus.is_ancestor(&entry))),
        "the find entry does not have focus"
    );
    bar.entry().set_text("line 79");
    let expected = doc.text.find("line 79").len();
    assert!(expected > 1, "the fixture has several matches");
    assert!(
        until(|| view.find_rects().len() == expected),
        "{} highlighted",
        view.find_rects().len()
    );

    // Next makes the next match current and brings it into view.
    let first = view.current_match().expect("a current match");
    view.find_step(true);
    let second = view.current_match().expect("still a current match");
    assert_ne!(first, second);
    let rect = doc.text.rects(second.clone())[0];
    let adjustment = scroller.vadjustment();
    assert!(
        rect.y0 >= adjustment.value() && rect.y1 <= adjustment.value() + adjustment.page_size(),
        "the current match is not in view"
    );
    view.find_step(false);
    assert_eq!(view.current_match(), Some(first));

    // A re-render keeps the highlights.
    let style = adw::StyleManager::default();
    let generation = doc.generation;
    style.set_color_scheme(adw::ColorScheme::ForceDark);
    assert!(until(|| view
        .document()
        .is_some_and(|d| d.generation > generation)));
    assert_eq!(
        view.find_rects().len(),
        expected,
        "a re-render dropped the highlights"
    );
    style.set_color_scheme(adw::ColorScheme::Default);

    // Escape closes the bar and clears them.
    bar.close();
    assert!(view.find_rects().is_empty(), "closing find left highlights");
    window.destroy();
}
