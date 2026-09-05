//! The settings panel's structured `[keys]` pane (#881): one row per
//! registered command, its current binding, and a capture button that
//! rebinds it to the next key pressed.
//!
//! `Section::Keys` used to only scroll the shared raw-text view to the
//! `[keys]` header; this is the structured row list next to it, the same
//! shape `redraw_filters` (#869), `redraw_ui` (#873) and `redraw_sync`
//! (#874) established. Skips without a display. Nothing here touches the
//! network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::SettingsPanel;
use postio_gtk::{fonts, style};

pub fn rows_render_one_per_command_with_its_current_binding() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    let row = row_for(&panel, "Next message");
    assert_eq!(binding_in(&row), "j", "the built-in default, unoverridden");

    window.destroy();
}

pub fn an_override_in_the_file_is_what_the_row_shows() {
    let Some((window, panel)) = panel_with_text("[keys]\nnext_message = \"n\"\n") else {
        return;
    };

    let row = row_for(&panel, "Next message");
    assert_eq!(binding_in(&row), "n");

    window.destroy();
}

pub fn capturing_a_free_key_writes_the_new_binding_to_the_buffer() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    let row = row_for(&panel, "Next message");
    rebind_button_in(&row).emit_clicked();
    pump();
    // Re-found rather than reused: `toggle_capture`'s redraw rebuilds every
    // row fresh, so the row fetched before the click is a detached widget
    // the panel has already replaced -- the same trap
    // `settings_accounts_token_wiring.rs` names for exactly this shape.
    let row = row_for(&panel, "Next message");
    assert_eq!(binding_in(&row), "press a key…");

    panel.test_capture_key(
        gdk::Key::from_name("n").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();

    let row = row_for(&panel, "Next message");
    assert_eq!(binding_in(&row), "n");
    assert!(
        panel.text().contains("next_message = \"n\""),
        "the capture must reach the buffer: {}",
        panel.text()
    );

    window.destroy();
}

pub fn capturing_a_binding_already_in_use_is_surfaced_not_silently_overwritten() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    let row = row_for(&panel, "Next message");
    rebind_button_in(&row).emit_clicked();
    pump();

    // "k" is PrevMessage's own default, and both share the List/Thread/
    // Reader/Search contexts -- a real collision.
    panel.test_capture_key(
        gdk::Key::from_name("k").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();

    let row = row_for(&panel, "Next message");
    assert_eq!(
        binding_in(&row),
        "j",
        "a conflicting capture must not overwrite the existing binding"
    );
    assert!(
        conflict_message_in(&row).is_some_and(|message| message.contains("Previous message")),
        "the conflict must name what it collides with"
    );
    assert!(
        !panel.text().contains("next_message"),
        "a rejected capture must never reach the buffer: {}",
        panel.text()
    );

    window.destroy();
}

pub fn escape_cancels_capture_without_changing_anything() {
    let Some((window, panel)) = panel_with_text("") else {
        return;
    };

    let row = row_for(&panel, "Next message");
    rebind_button_in(&row).emit_clicked();
    pump();

    panel.test_capture_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();

    let row = row_for(&panel, "Next message");
    assert_eq!(binding_in(&row), "j");
    assert!(!panel.text().contains("next_message"));

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

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-keys-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn row_for(panel: &SettingsPanel, title: &str) -> gtk::ListBoxRow {
    rows(panel)
        .into_iter()
        .find(|row| title_in(row) == title)
        .unwrap_or_else(|| panic!("no row for `{title}`"))
}

fn title_in(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-keys-title",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .expect("every keys row has a title label")
}

fn binding_in(row: &gtk::ListBoxRow) -> String {
    // The cap is the control now (#1179, Design/screens/22): pressing the
    // key a command is bound to is what changes it, so the key is a button
    // rather than a label with a separate `Rebind` beside it.
    rebind_button_in(row)
        .label()
        .map(|label| label.to_string())
        .expect("every keys row has a keycap")
}

fn conflict_message_in(row: &gtk::ListBoxRow) -> Option<String> {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-keys-conflict",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .filter(|label| label.is_visible())
    .map(|label| label.text().to_string())
}

fn rebind_button_in(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-keys-binding",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Button>().ok())
    .expect("every keys row has a keycap to press")
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget). Copied from `gtk_settings_sync.rs` rather than shared,
/// matching that file's own reason: no dependency between the two.
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
