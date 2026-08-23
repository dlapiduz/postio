//! The header's `Compose` button says `Composing` and `Esc` while the
//! composer has the reading pane, and pressing it then closes the composer
//! (keeping the draft) instead of opening a second one.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn the_compose_button_tracks_the_composer_and_closes_it_when_pressed_again() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-header-{}", std::process::id()));
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

    // Installing the composer is what wires the button at all — mirroring
    // `app.rs`'s own unconditional `window.composer()` call at startup.
    let composer = window.composer();
    let button = window.compose_button().expect("built alongside the header");

    assert_eq!(button.tooltip_text().as_deref(), Some("Compose a message"));

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();
    assert_eq!(
        button.tooltip_text().as_deref(),
        Some("Close the composer"),
        "the button should say what it now does, the moment the composer opens"
    );

    // ── Pressing it again closes rather than opening a second draft ───────
    composer.test_set_subject("Q3 numbers");
    settle();
    gtk::gio::prelude::ActionGroupExt::activate_action(&window, "compose", None);
    settle();
    assert!(
        !composer.is_open(),
        "the button should close an open composer instead of no-op'ing like `c` does"
    );
    assert_eq!(
        button.tooltip_text().as_deref(),
        Some("Compose a message"),
        "and say so again once it has"
    );

    // ── The draft was kept, exactly as Esc would have kept it ─────────────
    gtk::gio::prelude::ActionGroupExt::activate_action(&window, "compose", None);
    settle();
    assert_eq!(
        composer.draft().subject,
        "Q3 numbers",
        "closing the composer from the button must not discard what was typed"
    );
}
