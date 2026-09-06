//! The Sync & storage pane's controls (#1179, over #874's structured pane).
//!
//! The pane's dropdowns and its spin button are gone: a closed set of three
//! is a segmented control, a boolean is a checkbox, and a number is neither
//! (ADR 0029). What survives unchanged is where the values go — the same
//! format-preserving `patch_sync`, into the same buffer, on the same
//! debounced write. Skips without a display. Nothing here touches the
//! network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::{Section, SettingsPanel};
use postio_gtk::{fonts, style};

const SAMPLE: &str = "\
# a hand-written comment nobody wants to lose -- deliberately on an
# untouched section, not the sync table this pane owns.
[filters.old]
query = \"is:unread\"
pinned = true

[sync]
check_for_mail = \"poll\"
poll_interval_secs = 120
attachment_fetch = \"eager\"
notify = false
notify_roles = [\"inbox\", \"flagged\"]
";

pub fn the_pane_shows_the_files_values() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    assert!(
        segment(&panel, "Every 5 min").is_active(),
        "check_for_mail = \"poll\" is the middle segment"
    );
    assert!(
        segment(&panel, "Always").is_active(),
        "attachment_fetch = \"eager\" is the middle segment -- and a segment \
         rather than a checkbox because `never` is a third answer a box \
         could not say without silently rewriting it"
    );
    assert!(!checkbox(&panel, "Notify about new mail").is_active());
    assert_eq!(notify_roles(&panel).text(), "inbox, flagged");

    window.destroy();
}

pub fn the_interval_the_file_actually_holds_is_still_stated() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    // The spin button is gone, so this line is the only thing left that can
    // tell somebody their interval is 2 minutes rather than the 5 the
    // segment's label says. Rounding it away silently would be worse than
    // the dropdown that was here before.
    assert!(
        stat_lines(&panel).iter().any(|line| line.contains("2 min")),
        "the real interval must be stated, not rounded to the segment's \
         label: {:?}",
        stat_lines(&panel)
    );

    window.destroy();
}

pub fn pressing_manual_writes_straight_to_the_buffer_and_leaves_the_rest_alone() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    segment(&panel, "Manual").set_active(true);
    pump();

    let text = panel.text();
    let after_sync = text.split("[sync]").nth(1).expect("[sync] header");
    assert!(
        after_sync.contains("check_for_mail = \"manual\""),
        "pressing Manual must write check_for_mail = \"manual\": {text}"
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

pub fn an_interval_somebody_set_by_hand_survives_pressing_the_segment_it_is_on() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    // Already on `poll` at 120 seconds. Pressing the segment it is already
    // on is not a choice, and must not overwrite the interval with 300.
    segment(&panel, "Every 5 min").set_active(true);
    pump();

    assert!(
        panel.text().contains("poll_interval_secs = 120"),
        "pressing the chosen segment overwrote an interval set by hand: {}",
        panel.text()
    );

    window.destroy();
}

pub fn typing_new_roles_and_pressing_enter_writes_the_new_list() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let entry = notify_roles(&panel);
    entry.set_text("inbox, archive");
    entry.emit_activate();
    pump();

    let text = panel.text();
    assert!(
        text.contains("notify_roles = [\"inbox\", \"archive\"]"),
        "committing the field must write the new list: {text}"
    );

    window.destroy();
}

/// A panel showing `text`, with Sync & storage on screen — the pane builds
/// its controls on first display (#873), so it has to be shown first.
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
    panel.show_section(Section::Sync);
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

fn checkbox(panel: &SettingsPanel, label: &str) -> gtk::CheckButton {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-check")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::CheckButton>().ok())
        .find(|button| button.label().is_some_and(|text| text == label))
        .unwrap_or_else(|| panic!("no checkbox labelled {label:?}"))
}

fn notify_roles(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-notify-roles",
    )
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::Entry>().ok())
    .expect("the notify-roles field")
}

fn stat_lines(panel: &SettingsPanel) -> Vec<String> {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-stat-line")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
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
