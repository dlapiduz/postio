//! A draft keeps the Markdown it was written in only while it is written as
//! Markdown (docs/archive/specs/005-tui-frontend, data-model
//! `drafts.body_markdown`; spec 014 FR-028).
//!
//! A draft last edited as Markdown -- in the terminal, or the desktop's
//! Markdown view -- opens in the Markdown view with that text. Once the rich
//! view holds the body, that Markdown describes a message that no longer
//! exists, and reopening from it would silently undo the edit. So what the
//! rich view hands to be saved carries none, and the terminal falls back to
//! the HTML.

use crate::support_compose::Window;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::{AccountId, Draft};

use crate::support_compose::settle;

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
        composer.test_source_text().as_deref(),
        Some("Half a **sentence**"),
        "a draft written as Markdown did not open as that Markdown"
    );
    assert_eq!(
        composer.draft().body_markdown.as_deref(),
        Some("Half a **sentence**")
    );

    composer.dispatch(postio_core::CommandId::ToggleMarkdownSource);
    crate::support_compose::settle_until("the rich view", || composer.test_source_text().is_none());
    assert_eq!(
        composer.draft().body_markdown,
        None,
        "the rich view's copy of the draft still claims the Markdown"
    );
}
