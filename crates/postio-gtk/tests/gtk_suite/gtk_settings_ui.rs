//! The settings panel's structured `[ui]` pane (#873).
//!
//! `Section::Ui` used to be raw-TOML-textview-jump only; this is the
//! structured row list next to it, the same shape `redraw_filters`
//! established for `[filters]` (#869, `gtk_settings_filters.rs`). Skips
//! without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::SettingsPanel;
use postio_gtk::{fonts, style};

const SAMPLE: &str = "\
# a hand-written comment nobody wants to lose -- deliberately on an
# untouched section, not the appearance settings table this pane owns.
[filters.old]
query = \"is:unread\"
pinned = true

[ui]
theme = \"dark\"
density = \"compact\"
show_hover_actions = false
show_key_hints = true
sender_avatars = false
";

pub fn the_five_rows_render_from_a_given_config() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    assert_eq!(
        dropdown_in("Message-list row height", &panel).selected(),
        2,
        "compact"
    );
    assert_eq!(dropdown_in("Theme", &panel).selected(), 2, "dark");
    assert!(!switch_in("Show hover actions", &panel).is_active());
    assert!(switch_in("Show key hints", &panel).is_active());
    assert!(!switch_in("Show sender avatars", &panel).is_active());

    window.destroy();
}

pub fn the_default_config_renders_the_default_row_values() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    assert_eq!(
        dropdown_in("Message-list row height", &panel).selected(),
        0,
        "airy"
    );
    assert_eq!(dropdown_in("Theme", &panel).selected(), 0, "system");
    assert!(switch_in("Show hover actions", &panel).is_active());
    assert!(switch_in("Show sender avatars", &panel).is_active());

    window.destroy();
}

pub fn toggling_a_switch_writes_straight_to_the_buffer_and_leaves_everything_else_alone() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let switch = switch_in("Show sender avatars", &panel);
    assert!(!switch.is_active());
    switch.set_active(true);
    pump();

    let text = panel.text();
    let after_ui = text.split("[ui]").nth(1).expect("[ui] header");
    assert!(
        after_ui.contains("sender_avatars = true"),
        "toggling the switch must flip the value in the buffer: {text}"
    );
    assert!(
        text.contains("# a hand-written comment nobody wants to lose"),
        "an edit here must not disturb an unrelated section's comment: {text}"
    );
    assert!(
        text.contains("[filters.old]") && text.contains("query = \"is:unread\""),
        "nor an unrelated section: {text}"
    );

    window.destroy();
}

pub fn picking_a_theme_writes_the_new_value() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let dropdown = dropdown_in("Theme", &panel);
    dropdown.set_selected(1); // Light
    pump();

    let text = panel.text();
    let after_ui = text.split("[ui]").nth(1).expect("[ui] header");
    assert!(
        after_ui.contains("theme = \"light\""),
        "picking Light must write theme = \"light\": {text}"
    );

    window.destroy();
}

fn panel_with_text(text: &str) -> Option<(gtk::Window, SettingsPanel)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();
    panel.set_text(text);
    pump();
    Some((window, panel))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// The row whose title label reads `title`, if the ui pane has one.
fn row_named(panel: &SettingsPanel, title: &str) -> gtk::Box {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-ui-row")
        .into_iter()
        .find(|row| {
            collect(row, "postio-settings-ui-title")
                .into_iter()
                .find_map(|w| w.downcast::<gtk::Label>().ok())
                .is_some_and(|label| label.text() == title)
        })
        .unwrap_or_else(|| panic!("no [ui] row titled {title:?}"))
        .downcast()
        .expect("a Box row")
}

fn dropdown_in(title: &str, panel: &SettingsPanel) -> gtk::DropDown {
    let row = row_named(panel, title);
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::DropDown>().ok())
        .unwrap_or_else(|| panic!("row {title:?} has no dropdown"))
}

fn switch_in(title: &str, panel: &SettingsPanel) -> gtk::Switch {
    let row = row_named(panel, title);
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Switch>().ok())
        .unwrap_or_else(|| panic!("row {title:?} has no switch"))
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first. Copied from `gtk_settings_filters.rs` rather
/// than shared, matching that file's own reason: no dependency between the
/// two.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if class.is_empty() || widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}
