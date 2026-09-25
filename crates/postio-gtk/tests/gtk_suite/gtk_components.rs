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
        .filter(|widget| widget.has_css_class("postio-ghost"))
        .find(|widget| {
            widget
                .downcast_ref::<gtk::Button>()
                .and_then(|button| button.tooltip_text())
                .is_some_and(|tip| tip == "Keyboard shortcuts")
        })
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
