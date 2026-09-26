//! A draft the desktop saves forgets the Markdown it was written in
//! (specs/005-tui-frontend, data-model `drafts.body_markdown`).
//!
//! The terminal reopens a draft from its Markdown when there is some. Once the
//! desktop has edited the body, that Markdown describes a message that no
//! longer exists, and reopening from it would silently undo the edit. So what
//! the desktop composer hands to be saved carries none, and the terminal falls
//! back to the HTML.

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft};

use crate::settle;

pub fn a_draft_the_desktop_saves_carries_no_markdown() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    let composer = window.composer();
    window.present();

    let mut draft = Draft::new(AccountId::new(1));
    draft.body.text = Some("Half a sentence".to_owned());
    draft.body_markdown = Some("Half a **sentence**".to_owned());
    composer.open(draft);
    settle();

    assert_eq!(
        composer.draft().body_markdown,
        None,
        "the desktop's copy of the draft still claims the terminal's Markdown"
    );
}
