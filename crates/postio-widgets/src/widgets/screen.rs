//! A whole-window screen, and the window's chrome above it.
//!
//! Onboarding and the store-refused screen both replace the window's content
//! with a plate of their own, and the content is where this window keeps its
//! header bar — so each, in turn, left a window with no close button. The
//! wizard found it first (79770698); the refusal found it in the 0.4 Flatpak,
//! which is also where a refusal is usually met.
//!
//! The answer both use is here, so the next screen that replaces the content
//! starts with it: the screen goes under an `AdwToolbarView` whose top bar is
//! the window's title bar for as long as the screen is up. Flat and
//! title-less, because every such screen draws its own heading and a second
//! title would be two answers to "where am I"; but carrying the window
//! controls, which is the point. A header bar *inside* the screen was tried
//! and draws as part of the plate, a window inside a window.

use adw::prelude::*;

/// `screen` under the window's chrome, ready for `window.set_content`.
pub fn under_window_chrome(screen: &impl IsA<gtk::Widget>) -> adw::ToolbarView {
    let chrome = adw::ToolbarView::new();
    let bar = adw::HeaderBar::new();
    bar.set_show_title(false);
    bar.add_css_class("flat");
    chrome.add_top_bar(&bar);
    chrome.set_content(Some(screen));
    chrome
}

/// The `T` `window` is showing, wherever under its chrome it sits.
///
/// `window.content()` is the chrome rather than the screen, so reaching the
/// screen by downcasting the content finds nothing; this is what asks instead.
pub fn showing_in<T: IsA<gtk::Widget>>(window: &crate::window::Window) -> Option<T> {
    fn search<T: IsA<gtk::Widget>>(widget: &gtk::Widget) -> Option<T> {
        if let Ok(found) = widget.clone().downcast::<T>() {
            return Some(found);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = search(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    window.content().and_then(|content| search(&content))
}
