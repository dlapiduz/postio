use crate::support_compose::{fonts, style};
use crate::support_reader::collect;
use gtk::gdk;
use gtk::prelude::*;
use postio_widgets::reader::RemoteImageAllowList;
use postio_widgets::settings::SettingsPanel;

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

pub fn the_gnupg_switch_writes_the_config_asks_the_host_and_shows_what_it_read() {
    let Some((window, panel, _dir)) = panel_with_allowlist(RemoteImageAllowList::default()) else {
        return;
    };
    assert_eq!(
        panel.gnupg_keyring_line(),
        "Off: Postio reads no file outside its own store."
    );
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = asked.clone();
    panel.connect_gnupg_keyring(move |on| seen.borrow_mut().push(on));
    panel.turn_gnupg_keyring(true);
    pump();
    assert_eq!(*asked.borrow(), [true]);
    panel.reload_gnupg_keys();
    assert_eq!(
        *asked.borrow(),
        [true, true],
        "Reload GnuPG keys reads the files again"
    );
    assert!(
        postio_config::Config::from_toml_str(&panel.text())
            .expect("parses")
            .signatures
            .gnupg_keyring,
        "the switch is in the config: {}",
        panel.text()
    );
    panel.set_gnupg_keyring(&postio_ui::settings::GnupgKeyring::Read {
        keys: 312,
        skipped: 2,
    });
    assert_eq!(
        panel.gnupg_keyring_line(),
        "312 keys read from your GnuPG keyring · 2 skipped"
    );
    panel.set_text("");
    pump();
    assert_eq!(
        *asked.borrow(),
        [true, true],
        "a reloaded file moves the switch without asking"
    );
    window.destroy();
}
