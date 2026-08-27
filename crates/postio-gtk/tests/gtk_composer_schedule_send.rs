//! `ctrl+shift+Return` opens the same "Schedule send…" picker the pointer
//! reaches through the button beside Send — on a real display, since the
//! wiring runs through [`postio_core::CommandId::ScheduleSend`] dispatch and
//! a real key press.
//!
//! `schedule_presets` itself (which times the picker actually offers) is
//! unit-tested in `src/composer.rs` with no display; what needs one here is
//! that the key reaches [`postio_gtk::composer::Composer::dispatch`] at all,
//! and opens the same button clicking would.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

#[test]
fn ctrl_shift_return_opens_the_schedule_send_picker() {
    let state_dir = std::env::temp_dir().join(format!(
        "postio-composer-schedule-send-{}",
        std::process::id()
    ));
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

    let composer = window.composer();
    let button = composer.test_schedule_send_button();
    assert!(
        button.popover().is_none(),
        "nothing has asked for the picker yet"
    );

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    // Before the composer is addressed, `ctrl+shift+Return` must still open
    // the picker: choosing *when* to send is independent of whether the
    // draft is sendable yet, unlike `Send` itself.
    press(
        &window,
        "Return",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );

    assert!(
        button.popover().is_some_and(|popover| popover.is_visible()),
        "ctrl+shift+Return did not open the schedule-send picker"
    );
}
