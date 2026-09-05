//! The Appearance pane's controls (#1179, over #873's structured pane).
//!
//! The pane used to be built from a `gtk::DropDown` per closed choice and a
//! `gtk::Switch` per boolean. Both were the wrong control — a dropdown hides
//! the vocabulary it is choosing from, and a switch says "this takes effect
//! somewhere else" about a value that is simply a key in `config.toml`. This
//! asserts the pane is built from the ones that mean what they say, and that
//! they still write through the same format-preserving patch.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::{Section, SettingsPanel};
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

pub fn the_pane_shows_the_files_values_on_segments_and_checkboxes() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    assert!(
        segment(&panel, "Dark").is_active(),
        "theme = \"dark\" must show as the chosen segment"
    );
    assert!(!segment(&panel, "System").is_active());
    assert!(
        segment(&panel, "Compact").is_active(),
        "density = \"compact\" must show as the chosen segment"
    );

    assert!(!checkbox(&panel, "Hover action icons").is_active());
    assert!(checkbox(&panel, "Key hints on the focused row").is_active());
    assert!(!checkbox(&panel, "Sender avatars").is_active());

    window.destroy();
}

pub fn every_option_is_on_screen_without_opening_anything() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    // The whole reason a segmented control replaced a dropdown: a person
    // reading this pane can see that there are three answers, and what they
    // are, without pressing anything.
    for label in ["System", "Light", "Dark", "Airy", "Snug", "Compact"] {
        assert!(
            segment(&panel, label).is_visible(),
            "{label:?} must be on screen, not behind a dropdown"
        );
    }
    // Scoped to the pane on screen, not to the whole window: the accounts
    // pane's per-account enable toggle is a legitimate switch (it acts when
    // flipped — ADR 0029 Q2) and lives in the same widget tree.
    let pane = collect(panel.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::Stack>().ok())
        .and_then(|stack| stack.visible_child())
        .expect("a pane on screen");
    assert!(
        !collect(&pane, "")
            .into_iter()
            .any(|widget| widget.is::<gtk::DropDown>() || widget.is::<gtk::Switch>()),
        "no dropdown and no switch belongs on this pane"
    );

    window.destroy();
}

pub fn pressing_a_segment_writes_the_new_value_and_nothing_else() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    segment(&panel, "Light").set_active(true);
    pump();

    let text = panel.text();
    let after_ui = text.split("[ui]").nth(1).expect("[ui] header");
    assert!(
        after_ui.contains("theme = \"light\""),
        "pressing Light must write theme = \"light\": {text}"
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

pub fn ticking_a_checkbox_writes_straight_to_the_buffer() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let box_ = checkbox(&panel, "Sender avatars");
    assert!(!box_.is_active());
    box_.set_active(true);
    pump();

    let text = panel.text();
    let after_ui = text.split("[ui]").nth(1).expect("[ui] header");
    assert!(
        after_ui.contains("sender_avatars = true"),
        "ticking the box must flip the value in the buffer: {text}"
    );

    window.destroy();
}

pub fn the_density_line_says_what_the_choice_costs() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let line = collect(panel.upcast_ref::<gtk::Widget>(), "postio-stat-line")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .find(|label| label.text().contains("px rows"))
        .expect("the density line");
    let compact = line.text().to_string();

    segment(&panel, "Airy").set_active(true);
    pump();
    let airy = line.text().to_string();

    assert_ne!(
        compact, airy,
        "the line must be measured from the density, not printed once: {compact:?}"
    );
    assert!(
        airy.contains("px rows"),
        "and it must still say what it is: {airy:?}"
    );

    window.destroy();
}

/// A panel showing `text`, with the Appearance pane on screen.
///
/// The pane has to be *shown* before it has controls: every pane here
/// builds its own on first display rather than during `Window::new`, which
/// is #873's rule and the reason `show_section` exists.
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
    panel.show_section(Section::Appearance);
    pump();
    Some((window, panel))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// The segment labelled `label`.
fn segment(panel: &SettingsPanel, label: &str) -> gtk::ToggleButton {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-segment")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .find(|button| button.label().is_some_and(|text| text == label))
        .unwrap_or_else(|| panic!("no segment labelled {label:?}"))
}

/// The checkbox labelled `label`.
fn checkbox(panel: &SettingsPanel, label: &str) -> gtk::CheckButton {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-check")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::CheckButton>().ok())
        .find(|button| button.label().is_some_and(|text| text == label))
        .unwrap_or_else(|| panic!("no checkbox labelled {label:?}"))
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
