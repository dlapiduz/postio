//! The reading renderer's commands from the keyboard, through the window:
//! darken, zoom and find (spec 006 FR-013a, FR-018, FR-021), and the
//! fallback's "View source" (FR-023).
//!
//! Every key goes in through [`Window::handle_key`], for the reason
//! `gtk_reader_scroll` gives: a command that only works when called
//! directly proves nothing about whether a keystroke reaches it.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;
use postio_model::test_corpus;
use postio_render::Presentation;

fn press(window: &Window, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    window.handle_key(key, modifiers) == glib::Propagation::Stop
}

fn display() -> bool {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return false;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    true
}

pub fn the_keys_darken_zoom_and_find_in_the_message_on_screen() {
    if !display() {
        return;
    }
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceDark);
    let window = Window::default();
    window.set_default_size(900, 700);
    window.present();
    crate::pump();

    let zooms: Rc<RefCell<Vec<u16>>> = Rc::default();
    window.connect_zoom_changed({
        let zooms = Rc::clone(&zooms);
        move |percent| zooms.borrow_mut().push(percent)
    });

    // A newsletter on white paper: what darken is for.
    let parsed = postio_model::mime::parse(test_corpus::load("html-newsletter").bytes());
    window.show_message(&parsed.body, Some("weekly@news.example.org"));
    let reader = window.reader();
    crate::settle_until("the newsletter to be drawn on its paper", || {
        reader.view().document().is_some_and(|d| {
            d.messages
                .first()
                .is_some_and(|m| m.presentation == Presentation::Paper)
        })
    });

    // ── darken: `D`, and the palette's title is the undo ────────────────
    assert_eq!(reader.darken_title(), Some("Darken this message"));
    assert!(
        press(&window, gdk::Key::D, gdk::ModifierType::SHIFT_MASK),
        "D is claimed"
    );
    crate::settle_until("the message to be darkened", || {
        reader
            .view()
            .document()
            .is_some_and(|d| d.messages[0].presentation == Presentation::Darkened)
    });
    window.open_finder(Mode::Command);
    // A test window has no store behind it, and the palette offers nothing
    // that reads mail without one (#1114): say there is one.
    window
        .finder()
        .set_availability(postio_core::Availability::open(
            postio_core::Scope::default(),
        ));
    window.finder().set_query(Query {
        mode: Mode::Command,
        text: "darken".to_owned(),
    });
    crate::pump();
    assert!(
        window.finder().command_titles().contains(&"Show as sent"),
        "the palette still offers to darken a darkened message: {:?}",
        window.finder().command_titles()
    );
    // And the row does what it says: the undo, from the palette.
    assert_eq!(
        window.finder().commands().first(),
        Some(&postio_core::ActionId::Builtin(
            postio_core::CommandId::DarkenMessage
        ))
    );
    window.finder().activate();
    crate::pump();
    crate::settle_until("the message to be shown as sent again", || {
        reader
            .view()
            .document()
            .is_some_and(|d| d.messages[0].presentation == Presentation::Paper)
    });

    // ── zoom: the keys, the indicator, and what is persisted ────────────
    assert_eq!(reader.zoom_indicator(), None, "actual size says nothing");
    assert!(press(
        &window,
        gdk::Key::plus,
        gdk::ModifierType::CONTROL_MASK
    ));
    crate::pump();
    assert_eq!(reader.zoom(), 110, "one step in");
    assert_eq!(reader.zoom_indicator().as_deref(), Some("110%"));
    assert!(press(
        &window,
        gdk::Key::_0,
        gdk::ModifierType::CONTROL_MASK
    ));
    crate::pump();
    assert_eq!(reader.zoom(), 100, "and back to actual size");
    assert_eq!(reader.zoom_indicator(), None);
    assert_eq!(
        zooms.borrow().as_slice(),
        [110, 100],
        "each zoom a person chose is handed on to be persisted"
    );

    // Configuration applies to the reader open now and the next one built,
    // and is not written back to the file it came from.
    window.apply_reader(&postio_config::ReaderConfig {
        zoom: 150,
        ..Default::default()
    });
    crate::pump();
    assert_eq!(reader.zoom(), 150, "a config edit reaches the open reader");
    assert_eq!(window.new_reader().zoom(), 150, "and the next one");
    assert_eq!(zooms.borrow().len(), 2, "a configured zoom is not re-saved");
    window.apply_reader(&postio_config::ReaderConfig::default());
    crate::pump();

    // ── find: open, and the next match moves ─────────────────────────────
    assert!(press(&window, gdk::Key::f, gdk::ModifierType::CONTROL_MASK));
    crate::pump();
    assert!(reader.finding(), "mod+f opens find in the message");
    let view = reader.view().clone();
    view.set_find_query("the");
    crate::pump();
    let first = view.current_match();
    assert!(first.is_some(), "the newsletter has a \"the\" in it");
    assert!(press(&window, gdk::Key::g, gdk::ModifierType::CONTROL_MASK));
    crate::pump();
    assert_ne!(view.current_match(), first, "mod+g moves to the next match");

    style.set_color_scheme(adw::ColorScheme::Default);
    window.destroy();
}

/// The fallback notice's "View source" draws what was sent, as text.
pub fn view_source_draws_what_was_sent() {
    if !display() {
        return;
    }
    let reader = postio_gtk::reader::Reader::new(Rc::new(|_id: &str| None));
    let window = gtk::Window::new();
    window.set_default_size(600, 400);
    window.set_child(Some(&reader.widget()));
    window.present();
    crate::pump();
    reader.render(
        &MessageBody {
            text: None,
            html: Some("<table><tr><td>a cell</td></tr></table>".to_owned()),
        },
        Some("ada@example.com"),
    );
    crate::settle_until("the message to be drawn", || {
        reader
            .view()
            .document()
            .is_some_and(|d| d.text.text.contains("a cell"))
    });
    reader
        .view()
        .activate_action("body.view-source", None)
        .expect("the view offers View source");
    crate::settle_until("the source to be drawn", || {
        reader
            .view()
            .document()
            .is_some_and(|d| d.text.text.contains("<table><tr><td>a cell"))
    });
    window.destroy();
}
