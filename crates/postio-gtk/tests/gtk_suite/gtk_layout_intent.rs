//! The layout applies the viewport's constraint without rewriting the user's
//! intent (ADR 0024, #825).
//!
//! Two facts used to live in one boolean. `set_mode` wrote
//! `Shell::sidebar_visible` as *what the window can afford*, the header
//! toggle wrote it as *what the user asked for*, and `Window::save_state`
//! persisted whichever it was holding — so narrowing the window, quitting and
//! reopening wide left the sidebar gone with nothing to explain it.
//!
//! The unit tests in `shell.rs` pin the derivation itself. This drives a real
//! window through the modes, because the thing that broke was not the rule
//! but which authority got to write the property.
//!
//! No WebKit here on purpose — nothing opens a composer — so this belongs in
//! the shared-process suite rather than a binary of its own.
//!
//! Skips without a display.

use gtk::gdk;
use postio_gtk::shell::{Mode, Pane};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn a_narrow_window_hides_the_sidebar_and_widening_brings_it_back() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let shell = window.shell();
    assert!(shell.sidebar_visible(), "a wide window starts with one");
    assert!(shell.sidebar_wanted());

    // ── the viewport takes it away ────────────────────────────────────────
    shell.set_mode(Mode::TwoPane);
    assert!(!shell.sidebar_visible(), "no room for it at this width");
    assert!(
        shell.sidebar_wanted(),
        "but the user never said they did not want it"
    );

    // ── and gives it back ─────────────────────────────────────────────────
    shell.set_mode(Mode::ThreePane);
    assert!(
        shell.sidebar_visible(),
        "the width that hid it is the width that is now gone"
    );
}

pub fn a_sidebar_turned_off_stays_off_across_a_resize() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let shell = window.shell();

    // The user closes it at full width: that is a preference.
    shell.set_sidebar_visible(false);
    assert!(!shell.sidebar_wanted());

    // Narrowing and widening must not hand back something they asked to lose.
    shell.set_mode(Mode::MessageFocused);
    shell.set_mode(Mode::ThreePane);
    assert!(
        !shell.sidebar_visible(),
        "widening restored a sidebar the user had closed"
    );
    assert!(!shell.sidebar_wanted());
}

pub fn reaching_for_the_sidebar_on_a_narrow_window_is_not_a_preference() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let shell = window.shell();
    shell.set_sidebar_visible(false);
    shell.set_mode(Mode::TwoPane);

    // `shell.rs` promises the sidebar stays reachable in the narrower modes,
    // so this shows it — but for this window at this width only.
    shell.set_sidebar_visible(true);
    assert!(shell.sidebar_visible(), "the toggle still reaches it");
    assert!(
        !shell.sidebar_wanted(),
        "one look at the folder list on a small window is not a standing answer"
    );
}

pub fn opening_a_message_gives_the_reader_the_screen_when_there_is_room_for_one() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let shell = window.shell();
    shell.set_mode(Mode::MessageFocused);
    assert_eq!(
        shell.focused_pane(),
        Pane::List,
        "the list is where a narrow window starts"
    );

    // Below `MESSAGE_FOCUSED_WIDTH` there is room for one pane, and nothing
    // used to move this when a message opened -- so the reader filled and the
    // list stayed on screen.
    window.show_message(
        &postio_model::MessageBody {
            text: Some("The tide gate interlock is armed.".to_owned()),
            html: None,
        },
        Some("ada@example.com"),
    );
    assert_eq!(
        shell.focused_pane(),
        Pane::Reader,
        "opening a message left the list on screen"
    );
}
