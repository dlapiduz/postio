//! A composer built after a rebind starts on the rebound key (#828).
//!
//! One `#[test]`, alone in its binary, on purpose. It calls
//! `Window::composer`, which installs a WebKit editor, and
//! `tests/gtk_suite/` deliberately keeps WebKit out of the shared-process
//! suite — `gtk_reader` is excluded for the same reason (#272). Putting it
//! there destabilised the suite: `gtk_display_required`, which runs later and
//! asserts CI has a display at all, began failing.
//!
//! Its sibling case — that applying a keymap does *not* build a composer —
//! needs no WebKit and lives in `tests/gtk_suite/gtk_keymap_lazy.rs`.
//!
//! Skips without a display.

use gtk::gdk;
use postio_config::KeyBindings;
use postio_core::Keymap;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

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
