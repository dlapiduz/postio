//! The shared components in `postio_gtk::widgets`, measured the way a person
//! would see them rather than by what they were handed.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::{fonts, style, widgets};

/// A presented window to put components in, or `None` with no display.
fn window() -> Option<gtk::Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = gtk::Window::new();
    style::track(&window);
    Some(window)
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn width(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.measure(gtk::Orientation::Horizontal, -1).1
}

/// A kicker reads in capitals whatever case its source was written in, so a
/// pane that passes "Theme" and one that passes "THEME" draw the same thing.
pub fn a_kicker_is_capitals_whatever_its_source_case() {
    let Some(window) = window() else {
        return;
    };
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let written = widgets::kicker("Check for mail");
    let shouted = widgets::kicker("CHECK FOR MAIL");
    column.append(&written);
    column.append(&shouted);
    window.set_child(Some(&column));
    window.present();
    pump();

    assert_eq!(
        width(&written),
        width(&shouted),
        "a sentence-case kicker should render as the capitals the canvas draws"
    );
    window.destroy();
}

/// Every widget under `root`, depth first.
fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        out.push(widget.clone());
        out.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    out
}

/// The caps drawn inside the first widget wearing `class`.
fn caps_in(root: &gtk::Widget, class: &str) -> Vec<String> {
    let Some(owner) = descendants(root)
        .into_iter()
        .find(|widget| widget.has_css_class(class))
    else {
        panic!("nothing wears `{class}`");
    };
    descendants(&owner)
        .into_iter()
        .filter(|widget| widget.has_css_class("postio-keyhint"))
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.label().to_string())
        .collect()
}

/// A `[keys]` rebind reaches the header's caps, which used to be the
/// literals `c` and `?` and went on saying them after the keyboard had
/// moved (docs/PRODUCT.md §8: hints are derived, never typed in).
pub fn a_rebind_reaches_the_headers_key_caps() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = postio_gtk::window::Window::default();
    let root: gtk::Widget = window.clone().upcast();
    assert_eq!(caps_in(&root, "postio-compose"), vec!["c".to_string()]);

    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("compose".to_string(), "w".to_string());
    overrides
        .overrides_mut()
        .insert("cheat_sheet".to_string(), "F1".to_string());
    window.apply_keymap(postio_core::Keymap::resolve(&overrides));

    assert_eq!(
        caps_in(&root, "postio-compose"),
        vec!["w".to_string()],
        "the compose button still names the key compose no longer has"
    );
    let keys = descendants(&root)
        .into_iter()
        .find(|widget| widget.has_css_class("postio-header-keys"))
        .expect("the header's Keys button");
    let caps: Vec<String> = descendants(&keys)
        .into_iter()
        .filter(|widget| widget.has_css_class("postio-keyhint"))
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.label().to_string())
        .collect();
    assert_eq!(
        caps,
        vec!["F1".to_string()],
        "the Keys button missed the rebind"
    );
    window.destroy();
}

/// The four surfaces that float over the window and take the keyboard are
/// one plate: the same look, a dialog to a screen reader, and a title bar of
/// one height. They were five hand-built copies with headers of 42px and
/// 44px and three different roles.
pub fn every_overlay_is_one_plate() {
    let Some(window) = window() else {
        return;
    };
    let overlays: Vec<(&str, gtk::Widget)> = vec![
        (
            "cheat sheet",
            postio_gtk::cheatsheet::CheatSheet::new().upcast(),
        ),
        ("parts", postio_gtk::parts::PartsPanel::new().upcast()),
        (
            "unavailable",
            postio_gtk::unavailable::Unavailable::new().upcast(),
        ),
        (
            "onboarding",
            postio_gtk::onboarding::Onboarding::new().upcast(),
        ),
    ];
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (_, overlay) in &overlays {
        overlay.set_visible(true);
        column.append(overlay);
    }
    window.set_child(Some(&column));
    window.present();
    pump();

    let mut heights = Vec::new();
    for (name, overlay) in &overlays {
        assert!(
            overlay.has_css_class("postio-plate"),
            "the {name} is not a plate"
        );
        assert_eq!(
            overlay.accessible_role(),
            gtk::AccessibleRole::Dialog,
            "the {name} takes the keyboard, so it is a dialog"
        );
        let header = descendants(overlay)
            .into_iter()
            .find(|widget| widget.has_css_class("postio-plate-header"))
            .unwrap_or_else(|| panic!("the {name} has no plate header"));
        heights.push((*name, header.height()));
    }
    let first = heights[0].1;
    assert!(
        first >= 44,
        "a plate header is the canvas' 44px: {heights:?}"
    );
    assert!(
        heights.iter().all(|(_, height)| *height == first),
        "plate headers disagree on their height: {heights:?}"
    );

    let finder = postio_gtk::finder::Finder::new();
    assert!(
        finder.has_css_class("postio-plate"),
        "the box's results hang on the same surface"
    );
    window.destroy();
}

/// One primary look: every surface's "this is the one" verb wears the same
/// kind, and none still wears libadwaita's pill. It had been the pill in the
/// header, a pale tint in the action bars and a solid square in settings.
pub fn every_primary_button_is_the_same_kind() {
    let Some(window) = window() else {
        return;
    };
    let app_window = postio_gtk::window::Window::default();
    let roots: Vec<(&str, gtk::Widget)> = vec![
        ("window", app_window.clone().upcast()),
        ("parts", postio_gtk::parts::PartsPanel::new().upcast()),
        (
            "unavailable",
            postio_gtk::unavailable::Unavailable::new().upcast(),
        ),
        (
            "onboarding",
            postio_gtk::onboarding::Onboarding::new().upcast(),
        ),
    ];
    let mut primaries = 0;
    for (name, root) in &roots {
        for widget in descendants(root) {
            assert!(
                !widget.has_css_class("suggested-action"),
                "{name} still draws libadwaita's primary: {widget:?}"
            );
            if widget.has_css_class("postio-button-primary") {
                primaries += 1;
                assert!(
                    widget.has_css_class("postio-button"),
                    "{name}'s primary skipped the shared base"
                );
            }
        }
    }
    assert!(primaries >= 4, "found only {primaries} primaries");
    app_window.destroy();
    window.destroy();
}
