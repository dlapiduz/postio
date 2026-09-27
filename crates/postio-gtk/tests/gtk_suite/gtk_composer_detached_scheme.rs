//! A detached composer's window follows the scheme (specs/007-postio-focus
//! T025, research R1).
//!
//! The classic stylesheet keys its dark and high-contrast roles off
//! `postio-dark` and `postio-hc` on a window's root, and `style::track` keeps
//! those in step with `AdwStyleManager` -- for the main window and the
//! settings window. The composer's own window, built when it is detached,
//! was never tracked, so in dark mode it stayed light: a white composer
//! beside a dark application. What is asserted is the class the stylesheet
//! reads on the detached window's root.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{AccountId, Draft};

use crate::settle;

pub fn a_detached_composer_follows_dark_mode() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    let _ = fonts::install();
    style::install(&display);
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(adw::ColorScheme::ForceLight);

    let window = Window::default();
    window.present();
    settle();
    let composer = composer::install(&window);
    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();
    composer.detach();
    settle();
    let detached = composer
        .detached_window()
        .expect("the composer is in a window of its own");

    manager.set_color_scheme(adw::ColorScheme::ForceDark);
    settle();
    assert!(
        window.has_css_class(style::DARK_CLASS),
        "the main window is not dark, so this proves nothing about the other"
    );
    assert!(
        detached.has_css_class(style::DARK_CLASS),
        "the application went dark and the detached composer stayed light"
    );

    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    settle();
    assert!(
        !detached.has_css_class(style::DARK_CLASS),
        "the application went light again and the detached composer stayed dark"
    );

    manager.set_color_scheme(adw::ColorScheme::Default);
    composer.attach();
    window.destroy();
}
