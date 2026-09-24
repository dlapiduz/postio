//! The sidebar reaches Contacts (specs/005-contacts FR-001): a row of its
//! own below the saved searches, walked to with the keyboard like any other
//! and opened by landing on it, as a folder is.

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Command;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn walking_to_the_contacts_row_opens_contacts() {
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

    // No folders yet, so the Contacts row is the whole walk.
    assert!(!window.contacts_open());
    window.sidebar().step(1);
    pump();
    assert!(
        window.contacts_open(),
        "landing on the Contacts row opens it, the way landing on a folder opens the folder"
    );

    window.act(Command::Back);
    pump();
    assert!(!window.contacts_open(), "and Esc closes it again");

    window.destroy();
}
