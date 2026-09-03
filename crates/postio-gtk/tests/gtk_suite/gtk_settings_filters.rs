//! The settings panel's structured `[filters]` pane (#869).
//!
//! `Section::Filters` used to be raw-TOML-textview-jump only; this is the
//! rendered row list layered above it, the same shape `set_accounts` already
//! established for `[accounts]` (`gtk_settings_accounts.rs`). Unlike
//! accounts, `[filters]` lives entirely in `config.toml`, so this pane reads
//! and writes the panel's own buffer directly rather than needing an
//! external host: every row action is expected to reach `panel.text()`
//! through `postio_config::patch_filters`, and from there the panel's
//! existing debounced write already covers getting it to disk (proven in
//! `gtk_settings.rs`). Skips without a display. Nothing here touches the
//! network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::SettingsPanel;
use postio_gtk::{fonts, style};

const SAMPLE: &str = "\
# a hand-written comment nobody wants to lose
[ui]
theme = \"dark\" # inline comment, also not to be lost

[filters.needs-reply]
query = \"is:unread from:team\"
pinned = true
order = 0

[filters.has-attach]
query = \"has:attach\"
pinned = true
order = 1

[filters.archived-search]
query = \"is:archived\"
pinned = false
";

pub fn filters_render_as_rows_and_hide_when_there_are_none() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    assert!(rows(&panel).is_empty(), "no filters, no rows");
    assert!(
        !section_visible(&panel),
        "an empty filters section should not show at all"
    );
    assert!(
        empty_state_visible(&panel),
        "an empty filters section should say why, not just vanish"
    );

    panel.set_text(SAMPLE);
    pump();

    assert!(section_visible(&panel));
    assert!(!empty_state_visible(&panel));
    // Two pinned, in their explicit order, then the one unpinned filter.
    assert_eq!(rows(&panel).len(), 3, "one row per filter, pinned or not");

    window.destroy();
}

pub fn pinned_filters_come_first_in_order_then_unpinned_ones_alphabetically() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let titles: Vec<String> = rows(&panel).iter().map(query_text).collect();
    assert_eq!(
        titles,
        vec![
            "is:unread from:team".to_string(),
            "has:attach".to_string(),
            "is:archived".to_string(),
        ],
        "pinned filters in their explicit order, then unpinned ones",
    );

    window.destroy();
}

pub fn toggling_pinned_writes_straight_to_the_buffer() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let archived_row = rows(&panel)
        .into_iter()
        .find(|row| query_text(row) == "is:archived")
        .expect("the unpinned row");
    let switch = pinned_switch_in(&archived_row);
    assert!(!switch.is_active(), "archived-search starts unpinned");

    switch.set_active(true);
    pump();

    assert!(
        panel.text().contains("[filters.archived-search]") && {
            let text = panel.text();
            let after = text.split("[filters.archived-search]").nth(1).unwrap();
            after.contains("pinned = true")
        },
        "toggling the switch must flip pinned in the buffer: {}",
        panel.text()
    );
    assert!(
        panel
            .text()
            .contains("# a hand-written comment nobody wants to lose"),
        "an edit here must not disturb an unrelated section's comment: {}",
        panel.text()
    );
    assert!(
        panel
            .text()
            .contains("theme = \"dark\" # inline comment, also not to be lost"),
        "nor its own inline comment: {}",
        panel.text()
    );

    window.destroy();
}

pub fn deleting_a_filter_removes_its_row_and_leaves_everything_else_alone() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };
    assert_eq!(rows(&panel).len(), 3);

    let target = rows(&panel)
        .into_iter()
        .find(|row| query_text(row) == "has:attach")
        .expect("the has:attach row");
    delete_button_in(&target).emit_clicked();
    pump();

    assert_eq!(rows(&panel).len(), 2, "the deleted row is gone");
    assert!(!panel.text().contains("has-attach"));
    assert!(
        panel.text().contains("is:unread from:team"),
        "deleting one filter must not touch another: {}",
        panel.text()
    );
    assert!(
        panel
            .text()
            .contains("# a hand-written comment nobody wants to lose"),
        "nor an unrelated comment: {}",
        panel.text()
    );

    window.destroy();
}

pub fn reordering_moves_a_pinned_filter_and_disables_at_the_ends() {
    let Some((window, panel)) = panel_with_text(SAMPLE) else {
        return;
    };

    let first = &rows(&panel)[0];
    assert!(
        !up_button_in(first).is_sensitive(),
        "the first pinned filter has nowhere to move up to"
    );
    assert!(down_button_in(first).is_sensitive());

    down_button_in(first).emit_clicked();
    pump();

    let titles: Vec<String> = rows(&panel)
        .iter()
        .filter(|row| query_text(row) != "is:archived")
        .map(query_text)
        .collect();
    assert_eq!(
        titles,
        vec!["has:attach".to_string(), "is:unread from:team".to_string()],
        "moving the first pinned filter down should swap it with the second"
    );

    let archived = rows(&panel)
        .into_iter()
        .find(|row| query_text(row) == "is:archived")
        .expect("the unpinned row");
    assert!(
        !up_button_in(&archived).is_sensitive() && !down_button_in(&archived).is_sensitive(),
        "an unpinned filter is not part of the sidebar order, so it cannot be moved"
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

fn query_text(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-query",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .expect("every filter row has a query label")
}

fn pinned_switch_in(row: &gtk::ListBoxRow) -> gtk::Switch {
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Switch>().ok())
        .expect("every filter row has a pinned switch")
}

fn up_button_in(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(row.upcast_ref::<gtk::Widget>(), "postio-settings-filter-up")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Button>().ok())
        .expect("every filter row has an up button")
}

fn down_button_in(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-down",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Button>().ok())
    .expect("every filter row has a down button")
}

fn delete_button_in(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-delete",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Button>().ok())
    .expect("every filter row has a delete button")
}

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-filter-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn section_visible(panel: &SettingsPanel) -> bool {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-filters")
        .into_iter()
        .any(|w| w.is_visible())
}

fn empty_state_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-filters-empty",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first. Copied from `gtk_settings_accounts.rs` rather
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
