//! The Filtering pane's "After you unsubscribe" control (#1952): it shows
//! what `[focus] after_unsubscribe` holds and writes that one key, through
//! the same buffer every pane edits. Skips without a display.

use crate::support_compose::{fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::settings::{Section, SettingsPanel};

const SAMPLE: &str = "\
# kept
[ui]
density = \"compact\"

[focus]
filtering = true
";

pub fn the_control_defaults_to_archive_and_choosing_delete_writes_one_key() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };
    assert!(
        segment(&panel, "Archive").is_active(),
        "archive is the default"
    );

    segment(&panel, "Delete").set_active(true);
    pump();
    let text = panel.text();
    assert!(text.contains("after_unsubscribe = \"delete\""), "{text}");
    assert!(
        text.starts_with("# kept\n"),
        "the rest is untouched: {text}"
    );
    assert!(text.contains("density = \"compact\""), "{text}");

    segment(&panel, "Archive").set_active(true);
    pump();
    assert!(
        panel.text().contains("after_unsubscribe = \"archive\""),
        "{}",
        panel.text()
    );
    window.destroy();
}

pub fn a_file_that_says_delete_shows_delete() {
    let Some((window, panel)) = panel_with_text("[focus]\nafter_unsubscribe = \"delete\"\n") else {
        return;
    };
    assert!(segment(&panel, "Delete").is_active());
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
    panel.show_section(Section::Filtering);
    pump();
    Some((window, panel))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

fn segment(panel: &SettingsPanel, label: &str) -> gtk::ToggleButton {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-segment")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .find(|button| button.label().is_some_and(|text| text == label))
        .unwrap_or_else(|| panic!("no segment labelled {label:?}"))
}

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
