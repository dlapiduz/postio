use crate::support_compose::Window;
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_model::{AccountId, Draft};

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

pub fn ctrl_shift_return_opens_the_schedule_send_picker() {
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

    // Choosing when to send is independent of whether the draft is sendable yet, unlike `Send`.
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
