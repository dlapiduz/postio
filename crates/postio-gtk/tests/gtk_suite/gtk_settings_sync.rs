//! The settings panel's structured `[sync]` pane (#874).
//!
//! `Section::Sync` used to only scroll the shared raw-text view to the
//! `[sync]` header; this is the structured row list next to it, the same
//! shape `redraw_filters` (#869) and `redraw_ui` (#873) established. Skips
//! without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::SettingsPanel;
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

pub fn the_rows_render_from_a_given_config() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    assert_eq!(dropdown_in("Check for mail", &panel).selected(), 1, "poll");
    assert_eq!(spin_in("Poll interval (seconds)", &panel).value(), 120.0);
    assert_eq!(
        dropdown_in("Download attachments", &panel).selected(),
        1,
        "eager"
    );
    assert!(!switch_in("Notify for new mail", &panel).is_active());
    assert_eq!(
        entry_in("Notify for new mail", &panel).text(),
        "inbox, flagged"
    );

    window.destroy();
}

pub fn the_default_config_renders_the_default_row_values() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    assert_eq!(dropdown_in("Check for mail", &panel).selected(), 0, "idle");
    assert_eq!(spin_in("Poll interval (seconds)", &panel).value(), 300.0);
    assert!(switch_in("Notify for new mail", &panel).is_active());
    assert_eq!(entry_in("Notify for new mail", &panel).text(), "inbox");

    window.destroy();
}

pub fn picking_manual_writes_straight_to_the_buffer_and_leaves_everything_else_alone() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let dropdown = dropdown_in("Check for mail", &panel);
    dropdown.set_selected(2); // Manual
    pump();

    let text = panel.text();
    let after_sync = text.split("[sync]").nth(1).expect("[sync] header");
    assert!(
        after_sync.contains("check_for_mail = \"manual\""),
        "picking Manual must write check_for_mail = \"manual\": {text}"
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

pub fn typing_new_roles_and_pressing_enter_writes_the_new_list() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let entry = entry_in("Notify for new mail", &panel);
    entry.set_text("inbox,  archive ,inbox");
    entry.emit_by_name::<()>("activate", &[]);
    pump();

    let text = panel.text();
    let after_sync = text.split("[sync]").nth(1).expect("[sync] header");
    assert!(
        after_sync.contains("notify_roles = [\"inbox\", \"archive\", \"inbox\"]"),
        "each comma-separated role, trimmed, in order: {text}"
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

/// The row whose title label reads `title`, if the sync pane has one.
fn row_named(panel: &SettingsPanel, title: &str) -> gtk::Box {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-ui-row")
        .into_iter()
        .find(|row| {
            collect(row, "postio-settings-ui-title")
                .into_iter()
                .find_map(|w| w.downcast::<gtk::Label>().ok())
                .is_some_and(|label| label.text() == title)
        })
        .unwrap_or_else(|| panic!("no [sync] row titled {title:?}"))
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

fn spin_in(title: &str, panel: &SettingsPanel) -> gtk::SpinButton {
    let row = row_named(panel, title);
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::SpinButton>().ok())
        .unwrap_or_else(|| panic!("row {title:?} has no spin button"))
}

fn entry_in(title: &str, panel: &SettingsPanel) -> gtk::Entry {
    let row = row_named(panel, title);
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap_or_else(|| panic!("row {title:?} has no entry"))
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first. Copied from `gtk_settings_ui.rs` rather than
/// shared, matching that file's own reason: no dependency between the two.
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
