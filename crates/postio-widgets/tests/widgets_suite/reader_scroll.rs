//! A keyboard way to move through a message longer than one screen (#438):
//! the reader pages itself a screen at a time, from the top, and a new
//! message starts at its top.
//!
//! What is asserted is the reader's scroll offset, the thing a person would
//! call "did it scroll". The classic window's key routing (Page_Down,
//! space, shift+space reaching `Window::handle_key`, the context staying on
//! the list) is not here: it is the window's, not the reader's.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::MessageBody;
use postio_widgets::reader::{Reader, RemoteImageAllowList};

use crate::support_reader::{pump, settle_until};

fn body() -> MessageBody {
    MessageBody {
        text: Some("A message long enough that scrolling it means something. ".repeat(600)),
        html: None,
    }
}

/// A reader in a presented window, on a scratch allow list.
fn reader_in_a_window() -> Option<(gtk::Window, Reader, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    crate::support_reader::prepare(&gdk::Display::default().unwrap());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        dir.path().join("allow.json"),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();
    Some((window, reader, dir))
}

/// How far down the reader is scrolled.
fn scrolled(reader: &Reader) -> f64 {
    reader.scrolled_for_test()
}

/// Wait until the open message is drawn tall enough to page through.
fn wait_drawn(reader: &Reader) {
    settle_until("the message to be drawn tall enough to page", || {
        let view = reader.view().clone();
        view.document()
            .is_some_and(|d| d.size.height > 3.0 * f64::from(view.height().max(1)))
            && view.height() > 0
    });
}

/// Page down (or up) and wait for the offset to satisfy `moved`, returning it.
fn page(reader: &Reader, down: bool, moved: impl Fn(f64) -> bool) -> f64 {
    if down {
        reader.page_down();
    } else {
        reader.page_up();
    }
    settle_until("the reader to scroll", || moved(scrolled(reader)));
    scrolled(reader)
}

pub fn page_down_and_page_up_move_a_screen_at_a_time() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    reader.render(&body(), Some("ada@example.com"));
    wait_drawn(&reader);
    assert_eq!(
        scrolled(&reader),
        0.0,
        "a freshly rendered message starts at the top"
    );

    let one = page(&reader, true, |y| y > 0.0);
    let two = page(&reader, true, |y| y > one);
    let back = page(&reader, false, |y| y < two);
    assert!(
        (back - one).abs() < 1.0,
        "Page_Up came back to {back}, not {one}"
    );

    // Page_Up cannot go past the top.
    page(&reader, false, |y| y == 0.0);
    page(&reader, false, |y| y == 0.0);
    assert_eq!(
        scrolled(&reader),
        0.0,
        "Page_Up at the top stays at the top"
    );
    window.destroy();
}

pub fn a_new_message_resets_the_scroll_position() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    reader.render(&body(), Some("ada@example.com"));
    wait_drawn(&reader);
    let one = page(&reader, true, |y| y > 0.0);
    page(&reader, true, |y| y > one);

    // A second message starts at its top, not wherever the last one was.
    let generation = reader.view().document().map(|d| d.generation);
    reader.render(&body(), Some("grace@example.com"));
    settle_until("the second message to be drawn", || {
        reader.view().document().map(|d| d.generation) != generation
    });
    assert_eq!(
        scrolled(&reader),
        0.0,
        "the new message starts at its top, same as any fresh render"
    );
    let again = page(&reader, true, |y| y > 0.0);
    assert!(
        (again - one).abs() < 1.0,
        "paging the new message starts from its top: {again}, not {one}"
    );
    window.destroy();
}

pub fn paging_with_nothing_open_does_nothing() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    reader.page_down();
    pump();
    assert_eq!(
        scrolled(&reader),
        0.0,
        "nothing is open, so paging must not have scrolled anything"
    );
    window.destroy();
}
