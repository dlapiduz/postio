//! Loading the generated design tokens, and keeping them in step with the
//! system's light / dark / high-contrast preference.
//!
//! GTK evaluates `@media (prefers-color-scheme: dark)` only for the theme
//! provider, which it loads with an explicit variant; in an application
//! provider the query never matches, and libadwaita puts no class on the
//! widget tree either (both verified against GTK 4.22.4 / libadwaita 1.9.3).
//! So `tokens.css` keys its dark and high-contrast blocks off
//! `:root.postio-dark` and `:root.postio-hc`, and [`track`] puts those classes
//! on each window, following `AdwStyleManager`.
//!
//! Everything else — the libadwaita named colours the tokens override — keeps
//! working exactly as it does for any GNOME application.

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::resources;

/// The class `tokens.css` uses for its dark block.
pub const DARK_CLASS: &str = "postio-dark";

/// The class `tokens.css` uses for its high-contrast block.
pub const HIGH_CONTRAST_CLASS: &str = "postio-hc";

/// Load the generated tokens for `display`, then Postio's own widget styles.
///
/// Two sheets, in this order and at this priority: `tokens.css` is generated
/// from the design system and defines the variables, `shell.css` is written by
/// hand and dresses the widgets in them. Returns the providers so a caller can
/// drop them again; the app normally just leaves them installed for the life
/// of the process.
pub fn install(display: &gdk::Display) -> Vec<gtk::CssProvider> {
    resources::register();
    [resources::TOKENS_CSS, resources::SHELL_CSS]
        .into_iter()
        .map(|sheet| load(display, sheet))
        .collect()
}

fn load(display: &gdk::Display, sheet: &'static str) -> gtk::CssProvider {
    let name = sheet.rsplit('/').next().unwrap_or(sheet);
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(move |_, section, error| {
        // A parse error here means a sheet used something GTK's CSS subset
        // does not have. Loud, because the UI would be subtly wrong.
        glib::g_critical!("postio", "{name}: {}: {error}", section.to_str());
    });
    provider.load_from_resource(sheet);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    provider
}

/// Keep `root`'s scheme classes in step with the system preference.
///
/// Call this for every window: GTK's `:root` is the root widget, so the
/// scheme-dependent variables have to land there.
pub fn track(root: &impl IsA<gtk::Widget>) {
    let root = root.as_ref().clone();
    let manager = adw::StyleManager::default();

    apply(&manager, &root);

    let dark = manager.connect_dark_notify(glib::clone!(
        #[weak]
        root,
        move |manager| apply(manager, &root)
    ));
    let contrast = manager.connect_high_contrast_notify(glib::clone!(
        #[weak]
        root,
        move |manager| apply(manager, &root)
    ));

    let handlers = std::cell::RefCell::new(Some((dark, contrast)));
    root.connect_destroy(move |_| {
        if let Some((dark, contrast)) = handlers.borrow_mut().take() {
            let manager = adw::StyleManager::default();
            manager.disconnect(dark);
            manager.disconnect(contrast);
        }
    });
}

/// Install the tokens and track every window the application opens.
pub fn install_for_application(app: &impl IsA<gtk::Application>) {
    if let Some(display) = gdk::Display::default() {
        install(&display);
    }
    app.as_ref().connect_window_added(|_, window| track(window));
}

fn apply(manager: &adw::StyleManager, root: &gtk::Widget) {
    set_class(root, DARK_CLASS, manager.is_dark());
    set_class(root, HIGH_CONTRAST_CLASS, manager.is_high_contrast());
}

fn set_class(widget: &gtk::Widget, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}
