//! Applying a keymap does not build the composer (#828).
//!
//! `Window::composer` installs on first ask, deliberately: a window used only
//! for a test of the sidebar has no reason to pay for a WebKit editor. That
//! laziness is easy to defeat from something that runs on every window, and
//! `apply_keymap` did exactly that while wiring the composer's key hints —
//! which surfaced as `app_suite` timing out under its 300s watchdog rather
//! than as anything that looked like a bug.
//!
//! So the contract has a test of its own: a keymap applies to whatever exists,
//! and a composer built afterwards still starts on the applied keymap rather
//! than on the registry defaults it was drawn with.
//!
//! Here rather than in a binary of its own because two display-needing
//! `#[test]`s in one file hand them to libtest's thread pool, GTK tolerates
//! one thread, and the loser returns through its own `no display` guard and
//! is reported as passing (#355).
//!
//! Its sibling — that a composer built *after* a rebind starts on the rebound
//! key — is not here, because building one starts a WebKit editor and this
//! harness deliberately keeps WebKit out (see `main.rs`, and #272). It lives
//! in `tests/gtk_composer_keymap.rs`, alone in its binary.
//!
//! Skips without a display.

use gtk::gdk;
use postio_config::KeyBindings;
use postio_core::Keymap;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn applying_a_keymap_does_not_build_a_composer_nobody_asked_for() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    assert!(
        !window.has_composer(),
        "a fresh window has not built one yet"
    );

    window.apply_keymap(Keymap::resolve(&KeyBindings::default()));

    assert!(
        !window.has_composer(),
        "applying a keymap built a composer -- every window that reads \
         config.toml now pays for a WebKit editor it may never show"
    );
}
