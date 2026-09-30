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

/// Whether `widget` is on the page a stack is showing, all the way up to
/// `root` -- a stack's other faces are there but not drawn.
fn shown_within(widget: &gtk::Widget, root: &gtk::Widget) -> bool {
    let mut current = widget.clone();
    while &current != root {
        let Some(parent) = current.parent() else {
            return true;
        };
        if let Some(stack) = parent.downcast_ref::<gtk::Stack>()
            && stack.visible_child().as_ref() != Some(&current)
        {
            return false;
        }
        current = parent;
    }
    true
}

/// The caps drawn inside the first widget wearing `class` -- the ones a
/// person would see, so a stack's hidden face does not count.
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
        .filter(|widget| shown_within(widget, &owner))
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

/// Every settings pane keeps one rhythm: a section heading sits one `S6`
/// below what came before it, or flush at the top. The panes had been
/// written with 18, 20 and 22, so three panes of one window had three.
pub fn every_settings_heading_keeps_one_rhythm() {
    let Some(window) = window() else {
        return;
    };
    let panel = postio_gtk::settings::SettingsPanel::new();
    window.set_child(Some(&panel));
    window.present();
    pump();
    // Panes are built the first time they are shown.
    for section in postio_gtk::settings::Section::ALL {
        panel.show_section(section);
        pump();
    }

    let allowed = [0, widgets::space::S6];
    let mut seen = 0;
    for widget in descendants(panel.upcast_ref()) {
        // The sidebar's group headings belong to the list's header func,
        // not to a pane's column.
        if !widget.has_css_class("postio-kicker")
            || widget.has_css_class("postio-settings-nav-heading")
        {
            continue;
        }
        seen += 1;
        let label = widget.downcast_ref::<gtk::Label>().unwrap().label();
        assert!(
            allowed.contains(&widget.margin_top()),
            "`{label}` sits {}px below its neighbour; the rhythm is {allowed:?}",
            widget.margin_top()
        );
    }
    assert!(seen >= 8, "found only {seen} section headings");
    window.destroy();
}

/// A type role resolves to its size in GTK: text set in
/// `var(--postio-text-title)` is the same width as text set in the literal
/// it names. A role that did not resolve would fall back to the inherited
/// size and be narrower, silently.
pub fn a_type_role_resolves_to_its_size() {
    let Some(window) = window() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".probe-role { font-size: var(--postio-text-title); }\n\
         .probe-literal { font-size: 1.3636rem; }",
    );
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let role = gtk::Label::new(Some("Inbox is empty"));
    role.add_css_class("probe-role");
    let literal = gtk::Label::new(Some("Inbox is empty"));
    literal.add_css_class("probe-literal");
    let inherited = gtk::Label::new(Some("Inbox is empty"));
    column.append(&role);
    column.append(&literal);
    column.append(&inherited);
    window.set_child(Some(&column));
    window.present();
    pump();

    assert_eq!(width(&role), width(&literal), "the role did not resolve");
    assert!(
        width(&role) > width(&inherited),
        "a title is larger than body text"
    );
    gtk::style_context_remove_provider_for_display(&gdk::Display::default().unwrap(), &provider);
    window.destroy();
}

/// The header's compose button is one width whether it says `Compose` or
/// `Composing`.
///
/// It swapped its label in place, so opening the composer widened the
/// button by the difference and moved everything packed beside it -- 39px
/// in the measured window, on every open and close.
pub fn the_compose_button_holds_its_width_while_composing() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = postio_gtk::window::Window::default();
    window.present();
    pump();
    let root: gtk::Widget = window.clone().upcast();
    let compose = descendants(&root)
        .into_iter()
        .find(|widget| widget.has_css_class("postio-compose"))
        .expect("the header's compose button");
    let idle = width(&compose);

    window
        .composer()
        .open(postio_model::Draft::new(postio_model::ids::AccountId::new(
            1,
        )));
    pump();
    assert!(window.composer().is_open(), "the composer did not open");
    let composing = width(&compose);
    assert_eq!(
        idle, composing,
        "the compose button is {idle}px idle and {composing}px while composing"
    );
    window.destroy();
}
