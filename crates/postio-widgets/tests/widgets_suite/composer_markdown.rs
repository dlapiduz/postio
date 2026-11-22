use crate::support_compose::Window;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::{AccountId, Draft};

use crate::support_compose::settle;

pub fn a_draft_the_desktop_saves_carries_no_markdown() {
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
