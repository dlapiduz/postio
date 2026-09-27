//! #438: a keyboard way to move through a message longer than one screen,
//! without moving the keyboard off the message list.
//!
//! Every key here goes in through [`Window::handle_key`], not by calling the
//! reader directly — the same reason `gtk_parts.rs` does, and the same bead
//! this suite keeps citing (`postio-14b`): a command that only works when
//! called directly proves nothing about whether a keystroke can reach it.
//!
//! What is asserted is the reader's scroll offset. Under WebKit there was
//! no scroll position this side of the process boundary, so these cases
//! counted `#pos-N` markers; the reader's body is a `GtkScrollable` now
//! (spec 006), and the offset is the thing a person would call "did it
//! scroll".
//!
//! Skips without a display. Nothing here touches the network.

use crate::pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::MessageBody;

fn press(window: &Window, key: gdk::Key) -> bool {
    window.handle_key(key, gdk::ModifierType::empty()) == glib::Propagation::Stop
}

fn press_shift(window: &Window, key: gdk::Key) -> bool {
    window.handle_key(key, gdk::ModifierType::SHIFT_MASK) == glib::Propagation::Stop
}

fn body() -> MessageBody {
    MessageBody {
        text: Some("A message long enough that scrolling it means something. ".repeat(600)),
        html: None,
    }
}

/// How far down the reader is scrolled.
fn scrolled(window: &Window) -> f64 {
    window.reader().scrolled_for_test()
}

/// Wait until the open message is drawn tall enough to page through.
fn wait_drawn(window: &Window) {
    crate::settle_until("the message to be drawn tall enough to page", || {
        let view = window.reader().view().clone();
        view.document()
            .is_some_and(|d| d.size.height > 3.0 * f64::from(view.height().max(1)))
            && view.height() > 0
    });
}

/// Press `key` (with shift if `shift`) and wait for the offset to satisfy
/// `moved`, returning it.
fn page(window: &Window, key: gdk::Key, shift: bool, moved: impl Fn(f64) -> bool) -> f64 {
    let claimed = if shift {
        press_shift(window, key)
    } else {
        press(window, key)
    };
    assert!(claimed, "{key:?} should be claimed, not passed through");
    crate::settle_until("the reader to scroll", || moved(scrolled(window)));
    scrolled(window)
}

pub fn page_down_and_page_up_move_a_screen_at_a_time() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    assert_eq!(
        window.context(),
        postio_core::Context::List,
        "the window starts in List, which is where this command has to work \
         -- reading a message never switches context away from it"
    );

    window.show_message(&body(), Some("ada@example.com"));
    wait_drawn(&window);
    assert!(
        window.reading(),
        "a message should be open before paging it"
    );
    assert_eq!(
        scrolled(&window),
        0.0,
        "a freshly rendered message starts at the top"
    );

    // -- Page_Down, the default binding, then Page_Up walks it back ---------
    let one = page(&window, gdk::Key::Page_Down, false, |y| y > 0.0);
    let two = page(&window, gdk::Key::Page_Down, false, |y| y > one);
    let back = page(&window, gdk::Key::Page_Up, false, |y| y < two);
    assert!(
        (back - one).abs() < 1.0,
        "Page_Up came back to {back}, not {one}"
    );

    // -- the space/shift+space alternates do the same thing -----------------
    let down = page(&window, gdk::Key::space, false, |y| y > back);
    assert!(
        (down - two).abs() < 1.0,
        "space is the alternate for scrolling down"
    );
    let up = page(&window, gdk::Key::space, true, |y| y < down);
    assert!(
        (up - one).abs() < 1.0,
        "shift+space is the alternate for scrolling up"
    );

    // -- Page_Up cannot go past the top --------------------------------
    page(&window, gdk::Key::Page_Up, false, |y| y == 0.0);
    page(&window, gdk::Key::Page_Up, false, |y| y == 0.0);
    assert_eq!(
        scrolled(&window),
        0.0,
        "Page_Up at the top stays at the top"
    );

    // -- and the keyboard never left the list -------------------------------
    assert_eq!(
        window.context(),
        postio_core::Context::List,
        "paging the reader must not have moved the keyboard context"
    );
}

pub fn a_new_message_resets_the_scroll_position() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    window.show_message(&body(), Some("ada@example.com"));
    wait_drawn(&window);
    let one = page(&window, gdk::Key::Page_Down, false, |y| y > 0.0);
    page(&window, gdk::Key::Page_Down, false, |y| y > one);

    // A second message starts at its top, not wherever the last one was.
    let generation = window.reader().view().document().map(|d| d.generation);
    window.show_message(&body(), Some("grace@example.com"));
    crate::settle_until("the second message to be drawn", || {
        window.reader().view().document().map(|d| d.generation) != generation
    });
    assert_eq!(
        scrolled(&window),
        0.0,
        "the new message starts at its top, same as any fresh render"
    );
    let again = page(&window, gdk::Key::Page_Down, false, |y| y > 0.0);
    assert!(
        (again - one).abs() < 1.0,
        "paging the new message starts from its top: {again}, not {one}"
    );
}

pub fn paging_with_nothing_open_does_nothing() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    assert!(!window.reading(), "nothing should be open yet");
    // The command is still claimed -- it is bound in this context whether
    // or not a message is open, the same way `j`/`k` are claimed with an
    // empty list. What must not happen is a scroll of nothing.
    press(&window, gdk::Key::Page_Down);
    pump();
    assert_eq!(
        scrolled(&window),
        0.0,
        "nothing is open, so paging must not have scrolled anything"
    );
}
