//! Undo toasts: `u` and the toast's own button reach the same
//! `CommandId::Undo`, and showing a completion does not crash the window.
//!
//! `AdwToastOverlay` does not expose which toasts are showing, so
//! `postio_gtk::toast`'s own unit tests are what prove coalescing and the
//! button's shape; what needs a real window is the "one path" claim itself —
//! that the keyboard and the mouse actually converge, not just that they are
//! supposed to.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn u_and_the_toasts_button_both_reach_command_id_undo() {
    let state_dir = std::env::temp_dir().join(format!("postio-undo-toast-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let seen: std::rc::Rc<std::cell::RefCell<Vec<CommandId>>> = Default::default();
    window.connect_command({
        let seen = std::rc::Rc::clone(&seen);
        move |id| seen.borrow_mut().push(id)
    });

    // ── `u`, through the keymap ────────────────────────────────────────
    window.handle_key(
        gdk::Key::from_name("u").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(
        *seen.borrow(),
        vec![CommandId::Undo],
        "u did not reach dispatch"
    );
    seen.borrow_mut().clear();

    // ── the toast's own button, through `win.undo` ─────────────────────
    // Showing the completion is also the smoke test that it does not panic
    // building a real toast against a real overlay.
    window.show_action_completed("Archived 12 messages", true);
    settle();
    gtk::gio::prelude::ActionGroupExt::activate_action(&window, "undo", None);
    settle();
    assert_eq!(
        *seen.borrow(),
        vec![CommandId::Undo],
        "the toast's button must reach the exact command `u` does, not a parallel path"
    );

    // ── a completion with nothing to undo, and undo's own confirmation ─
    // Neither should panic against a real ToastOverlay.
    window.show_action_completed("Marked 1 message as read", false);
    settle();
    window.show_undo_performed("Archived 12 messages, undone");
    settle();
}
