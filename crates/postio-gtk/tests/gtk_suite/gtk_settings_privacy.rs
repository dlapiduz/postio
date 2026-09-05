//! The settings panel's privacy pane (#871): the remote-image allow-list,
//! managed rather than only enforced.
//!
//! `RemoteImageAllowList` already exists and already persists
//! (`crates/postio-gtk/src/reader/allowlist.rs`) — this is the other half,
//! the same shape `gtk_settings_accounts.rs` proves for `set_accounts`:
//! given a list, does the pane draw one row per sender, and does revoking
//! one actually mutate and save it. Skips without a display. Nothing here
//! touches the network.

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::reader::RemoteImageAllowList;
use postio_gtk::settings::SettingsPanel;
use postio_gtk::{fonts, style};
use postio_model::UnsubscribeActivation;
use postio_model::ids::AccountId;

pub fn allowed_senders_render_as_rows_and_hide_when_there_are_none() {
    let Some((window, panel, _dir)) = panel_with_allowlist(RemoteImageAllowList::default()) else {
        return;
    };

    assert!(rows(&panel).is_empty(), "nobody allow-listed, no rows");
    assert!(
        !section_visible(&panel),
        "an empty privacy section should not show at all"
    );
    assert!(
        empty_state_visible(&panel),
        "an empty privacy section should say why, not just vanish"
    );

    window.destroy();
}

pub fn every_allowed_sender_gets_its_own_row() {
    let mut list = RemoteImageAllowList::default();
    list.allow("ada@example.com");
    list.allow("zed@example.com");
    let Some((window, panel, _dir)) = panel_with_allowlist(list) else {
        return;
    };

    assert!(section_visible(&panel));
    assert!(!empty_state_visible(&panel));
    let senders: Vec<String> = rows(&panel).iter().map(sender_text).collect();
    assert_eq!(senders, vec!["ada@example.com", "zed@example.com"]);

    window.destroy();
}

pub fn revoking_a_sender_removes_its_row_and_persists() {
    let mut list = RemoteImageAllowList::default();
    list.allow("ada@example.com");
    list.allow("zed@example.com");
    let Some((window, panel, dir)) = panel_with_allowlist(list) else {
        return;
    };
    let path = dir.path().join("remote-images.ini");

    let target = rows(&panel)
        .into_iter()
        .find(|row| sender_text(row) == "ada@example.com")
        .expect("ada's row");
    revoke_button_in(&target).emit_clicked();
    pump();

    let senders: Vec<String> = rows(&panel).iter().map(sender_text).collect();
    assert_eq!(
        senders,
        vec!["zed@example.com"],
        "revoking one sender must leave the other alone"
    );

    let reloaded = RemoteImageAllowList::load_from(&path);
    assert!(
        !reloaded.is_allowed("ada@example.com"),
        "a revoke must reach disk, not just the row"
    );
    assert!(
        reloaded.is_allowed("zed@example.com"),
        "and must not touch anyone else's exception"
    );

    window.destroy();
}

pub fn no_activations_hides_the_unsubscribe_section_and_shows_the_empty_state() {
    let Some((window, panel)) = panel() else {
        return;
    };

    panel.set_unsubscribe_activations(Vec::new());
    pump();

    assert!(
        unsubscribe_rows(&panel).is_empty(),
        "nothing was ever activated, no rows"
    );
    assert!(
        !unsubscribe_section_visible(&panel),
        "an empty log should not show at all"
    );
    assert!(
        unsubscribe_empty_state_visible(&panel),
        "an empty log should say why, not just vanish"
    );

    window.destroy();
}

pub fn every_activation_gets_its_own_row_newest_first() {
    let Some((window, panel)) = panel() else {
        return;
    };

    let account = AccountId::new(1);
    let older = UnsubscribeActivation::new(
        account,
        "old-newsletter.example.com",
        Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap(),
    );
    let newer = UnsubscribeActivation::new(
        account,
        "new-newsletter.example.com",
        Utc.with_ymd_and_hms(2026, 8, 30, 0, 0, 0).unwrap(),
    );
    // Handed in newest-first, the same order `UnsubscribeRepository::for_account`
    // already returns — the pane draws what it is given, it does not reorder.
    panel.set_unsubscribe_activations(vec![newer, older]);
    pump();

    assert!(unsubscribe_section_visible(&panel));
    assert!(!unsubscribe_empty_state_visible(&panel));
    let lists: Vec<String> = unsubscribe_rows(&panel)
        .iter()
        .map(unsubscribe_list_text)
        .collect();
    assert_eq!(
        lists,
        vec![
            "new-newsletter.example.com".to_owned(),
            "old-newsletter.example.com".to_owned(),
        ]
    );

    window.destroy();
}

pub fn the_read_receipt_count_states_zero_rather_than_going_blank() {
    let Some((window, panel)) = panel() else {
        return;
    };

    panel.set_read_receipt_count(0);
    pump();

    assert_eq!(
        panel.read_receipt_count_label(),
        "No messages have requested a read receipt.",
        "zero is itself the answer -- CLAUDE.md's privacy section makes this \
         a fixed policy, so there is nothing to hide behind an empty state"
    );

    window.destroy();
}

pub fn the_read_receipt_count_states_the_number_and_says_none_are_sent() {
    let Some((window, panel)) = panel() else {
        return;
    };

    panel.set_read_receipt_count(3);
    pump();

    let label = panel.read_receipt_count_label();
    assert!(
        label.contains('3'),
        "the count should be in the text: {label}"
    );
    assert!(
        label.contains("none have been sent"),
        "CLAUDE.md's privacy section makes never-automatic a fixed policy -- \
         the line states that fact, it does not offer a switch over it: {label}"
    );

    window.destroy();
}

fn panel() -> Option<(gtk::Window, SettingsPanel)> {
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
    pump();
    Some((window, panel))
}

fn unsubscribe_list_text(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-unsubscribe-list-identifier",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .expect("every unsubscribe row names a list")
}

fn unsubscribe_rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-unsubscribe-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn unsubscribe_section_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-unsubscribe",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

fn unsubscribe_empty_state_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-unsubscribe-empty",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

fn panel_with_allowlist(
    list: RemoteImageAllowList,
) -> Option<(gtk::Window, SettingsPanel, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("remote-images.ini");
    list.save_to(&path).expect("seed the scratch allow-list");

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();
    panel.set_remote_image_allowlist(RemoteImageAllowList::load_from(&path), path);
    pump();
    Some((window, panel, dir))
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

fn sender_text(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-privacy-sender",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .expect("every privacy row has a sender label")
}

fn revoke_button_in(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-privacy-revoke",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Button>().ok())
    .expect("every privacy row has a revoke button")
}

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-privacy-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn section_visible(panel: &SettingsPanel) -> bool {
    collect(panel.upcast_ref::<gtk::Widget>(), "postio-settings-privacy")
        .into_iter()
        .any(|w| w.is_visible())
}

fn empty_state_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-privacy-empty",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget). Copied from `gtk_settings_filters.rs` rather than shared,
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
