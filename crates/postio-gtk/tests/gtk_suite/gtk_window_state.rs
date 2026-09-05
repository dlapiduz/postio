//! What a window would persist, asserted without a file (#852).
//!
//! `save_state` decides what to write and then writes it to
//! `WindowState::path()` — the real user state file. That made the *decision*
//! untestable: overriding `XDG_STATE_HOME` here would leak into every other
//! case sharing this process, and writing to the developer's own state file
//! is not something a test gets to do.
//!
//! So the decision is [`Window::window_state`] and the write is `save_state`
//! on top of it. This asserts the decision.
//!
//! The field that matters is `sidebar_visible`, which holds the user's
//! *preference* rather than what this window's width can currently afford
//! (ADR 0024). Persisting the width-derived value is exactly what #825 was:
//! quit on a narrow window, reopen wide, and the sidebar was gone for good.
//!
//! No WebKit, so this belongs in the shared-process suite.
//!
//! Skips without a display.

use gtk::gdk;
use postio_gtk::shell::Mode;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

fn window() -> Option<Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return None;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    Some(Window::default())
}

pub fn narrowing_the_window_does_not_save_away_the_sidebar() {
    let Some(window) = window() else { return };
    let shell = window.shell();

    // The user wants the sidebar and the window is wide enough to show it.
    assert!(shell.sidebar_visible());

    // Now it is not wide enough. The sidebar goes; the preference does not.
    shell.set_mode(Mode::TwoPane);
    assert!(!shell.sidebar_visible(), "no room for it at this width");

    let state = window.window_state().expect("a built window has state");
    assert!(
        state.sidebar_visible,
        "quitting on a narrow window recorded the breakpoint's answer as the \
         user's preference, so reopening wide had no sidebar and nothing to \
         explain it (#825)"
    );
}

pub fn closing_the_sidebar_at_full_width_is_saved_as_a_preference() {
    let Some(window) = window() else { return };
    let shell = window.shell();

    // The other direction, or the fix above would just be "always true".
    shell.set_sidebar_visible(false);

    let state = window.window_state().expect("a built window has state");
    assert!(
        !state.sidebar_visible,
        "the user closed it at a width where that means something"
    );
}

pub fn what_a_window_would_save_survives_a_round_trip() {
    let Some(window) = window() else { return };
    let shell = window.shell();
    shell.set_sidebar_visible(false);
    shell.set_mode(Mode::TwoPane);

    let directory = tempfile::tempdir().expect("a temp directory");
    let path = directory.path().join("window.ini");
    let state = window.window_state().expect("a built window has state");
    state.save_to(&path).expect("write the state");

    let read_back = postio_gtk::state::WindowState::load_from(&path);
    assert_eq!(
        read_back.sidebar_visible, state.sidebar_visible,
        "the preference did not survive the file"
    );
    assert_eq!(read_back.width, state.width);
    assert_eq!(read_back.sidebar_width, state.sidebar_width);
}
