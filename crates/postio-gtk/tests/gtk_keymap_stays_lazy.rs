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
//! Skips without a display.

use gtk::gdk;
use gtk::prelude::*;
use postio_config::KeyBindings;
use postio_core::Keymap;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

#[test]
fn applying_a_keymap_does_not_build_a_composer_nobody_asked_for() {
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

#[test]
fn a_composer_built_after_a_rebind_starts_on_the_rebound_key() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    // The other half of keeping it lazy: `apply_keymap` cannot reach a
    // composer that does not exist, so the composer has to pick the keymap up
    // when it is finally built -- or a rebind made before anyone composed
    // would be invisible until the next edit of `config.toml`.
    let window = Window::default();
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("save_draft".to_string(), "mod+w".to_string());
    window.apply_keymap(Keymap::resolve(&overrides));

    let composer = window.composer();
    assert!(window.has_composer(), "asking for it builds it");
    assert_eq!(
        composer.test_save_hint(),
        Some("ctrl+w".to_string()),
        "the composer was built after the rebind and missed it"
    );
}
