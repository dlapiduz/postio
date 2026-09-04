//! The account detail view (#880): activating an account row opens a
//! structured, editable form over its real settings.
//!
//! ADR 0005 Q6b retired `[accounts]` from `config.toml` -- an account is
//! database state, not preference -- so this is not a `[table]` pane like
//! `[ui]`/`[sync]`/`[filters]`: there is no buffer to patch. The panel only
//! reports what changed (`connect_account_edited`); `settings_accounts.rs`
//! in `postio-app` is what actually writes it, the same split
//! `connect_account_enabled_changed`/`connect_account_action` already use.
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::settings::{AccountEdit, AccountMailboxes, SettingsPanel};
use postio_gtk::{fonts, style};
use postio_model::account::{ServerConfig, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, MailboxRole};

fn an_account(id: i64, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.id = AccountId::new(id);
    account.incoming = ServerConfig {
        host: "imap.example.com".to_owned(),
        port: 993,
        security: TransportSecurity::Tls,
        username: address.to_owned(),
    };
    account.outgoing = ServerConfig {
        host: "smtp.example.com".to_owned(),
        port: 587,
        security: TransportSecurity::StartTls,
        username: address.to_owned(),
    };
    account
}

pub fn activating_a_row_opens_the_detail_view_with_its_current_settings() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };

    assert!(!detail_visible(&panel), "closed until a row is activated");

    rows(&panel)[0].emit_activate();
    pump();

    assert!(detail_visible(&panel));
    assert_eq!(display_name_entry(&panel).text(), "Ada");
    assert_eq!(imap_host_entry(&panel).text(), "imap.example.com");
    assert_eq!(imap_port_spin(&panel).value() as u16, 993);
    assert_eq!(smtp_host_entry(&panel).text(), "smtp.example.com");
    assert_eq!(smtp_port_spin(&panel).value() as u16, 587);

    window.destroy();
}

pub fn the_back_button_returns_to_the_account_list() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    rows(&panel)[0].emit_activate();
    pump();
    assert!(detail_visible(&panel));

    back_button(&panel).emit_clicked();
    pump();

    assert!(!detail_visible(&panel));

    window.destroy();
}

pub fn editing_the_display_name_reports_the_account_and_the_new_value() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    rows(&panel)[0].emit_activate();
    pump();

    let seen: std::rc::Rc<std::cell::RefCell<Vec<(AccountId, AccountEdit)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    panel.connect_account_edited({
        let seen = seen.clone();
        move |id, edit| seen.borrow_mut().push((id, edit))
    });

    let entry = display_name_entry(&panel);
    entry.set_text("Ada Lovelace");
    entry.emit_activate();
    pump();

    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "exactly one edit for one commit: {seen:?}");
    assert_eq!(seen[0].0, AccountId::new(1));
    assert_eq!(
        seen[0].1,
        AccountEdit::DisplayName("Ada Lovelace".to_owned())
    );

    window.destroy();
}

pub fn editing_the_imap_port_reports_the_account_and_the_new_value() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    rows(&panel)[0].emit_activate();
    pump();

    let seen: std::rc::Rc<std::cell::RefCell<Vec<(AccountId, AccountEdit)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    panel.connect_account_edited({
        let seen = seen.clone();
        move |id, edit| seen.borrow_mut().push((id, edit))
    });

    imap_port_spin(&panel).set_value(143.0);
    pump();

    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "exactly one edit for one change: {seen:?}");
    assert_eq!(seen[0].0, AccountId::new(1));
    assert_eq!(seen[0].1, AccountEdit::ImapPort(143));

    window.destroy();
}

pub fn opening_a_second_account_populates_its_own_settings_not_the_firsts() {
    let Some((window, panel)) = panel_with_two_accounts() else {
        return;
    };

    rows(&panel)[1].emit_activate();
    pump();

    assert_eq!(display_name_entry(&panel).text(), "Grace");
    assert_eq!(imap_host_entry(&panel).text(), "imap.example.com");

    window.destroy();
}

fn panel_with_account() -> Option<(gtk::Window, SettingsPanel)> {
    let (window, panel) = new_panel()?;
    panel.set_accounts(vec![an_account(1, "Ada", "ada@example.com")]);
    pump();
    Some((window, panel))
}

fn panel_with_two_accounts() -> Option<(gtk::Window, SettingsPanel)> {
    let (window, panel) = new_panel()?;
    panel.set_accounts(vec![
        an_account(1, "Ada", "ada@example.com"),
        an_account(2, "Grace", "grace@example.com"),
    ]);
    pump();
    Some((window, panel))
}

fn new_panel() -> Option<(gtk::Window, SettingsPanel)> {
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

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn detail_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

fn back_button(panel: &SettingsPanel) -> gtk::Button {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-back",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Button>().ok())
    .expect("the detail view has a back button")
}

fn display_name_entry(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-display-name",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Entry>().ok())
    .expect("the detail view has a display name entry")
}

fn imap_host_entry(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-imap-host",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Entry>().ok())
    .expect("the detail view has an IMAP host entry")
}

fn imap_port_spin(panel: &SettingsPanel) -> gtk::SpinButton {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-imap-port",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::SpinButton>().ok())
    .expect("the detail view has an IMAP port spin button")
}

fn smtp_host_entry(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-smtp-host",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Entry>().ok())
    .expect("the detail view has an SMTP host entry")
}

fn smtp_port_spin(panel: &SettingsPanel) -> gtk::SpinButton {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-smtp-port",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::SpinButton>().ok())
    .expect("the detail view has an SMTP port spin button")
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget). Copied from `gtk_settings_accounts.rs` rather than shared,
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

// ── The Mailboxes group (ADR 0027, #966) ────────────────────────────────

/// What one account's folders and role map look like coming in.
fn folders() -> AccountMailboxes {
    AccountMailboxes {
        folders: vec![
            "INBOX".to_owned(),
            "Sent".to_owned(),
            "Sent Messages".to_owned(),
        ],
        chosen: Vec::new(),
        resolved: vec![(MailboxRole::Sent, "Sent".to_owned())],
    }
}

fn role_dropdown(panel: &SettingsPanel, role: MailboxRole) -> gtk::DropDown {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        &format!("postio-settings-account-detail-role-{}", role.as_str()),
    )
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::DropDown>().ok())
    .unwrap_or_else(|| panic!("the detail view has a {role:?} dropdown"))
}

fn entries(dropdown: &gtk::DropDown) -> Vec<String> {
    let model = dropdown
        .model()
        .expect("the dropdown has a model")
        .downcast::<gtk::StringList>()
        .expect("built from strings");
    (0..model.n_items())
        .map(|index| model.string(index).expect("an entry").to_string())
        .collect()
}

pub fn the_mailboxes_group_offers_automatic_first_and_names_what_it_resolved_to() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    panel.set_account_mailboxes(vec![(AccountId::new(1), folders())]);
    rows(&panel)[0].emit_activate();
    pump();

    let dropdown = role_dropdown(&panel, MailboxRole::Sent);
    assert_eq!(
        entries(&dropdown),
        vec![
            "Automatic (Sent)".to_owned(),
            "INBOX".to_owned(),
            "Sent".to_owned(),
            "Sent Messages".to_owned(),
        ],
        "automatic comes first and says what it picked, so a person can \
         disagree with an answer they can see"
    );
    assert_eq!(dropdown.selected(), 0, "nothing is chosen yet");

    window.destroy();
}

pub fn picking_a_folder_for_a_role_reports_the_account_and_the_path() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    panel.set_account_mailboxes(vec![(AccountId::new(1), folders())]);
    rows(&panel)[0].emit_activate();
    pump();

    let seen: std::rc::Rc<std::cell::RefCell<Vec<(AccountId, AccountEdit)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    panel.connect_account_edited({
        let seen = seen.clone();
        move |id, edit| seen.borrow_mut().push((id, edit))
    });

    // "Sent Messages" is the last of the three folders, so the third entry
    // after Automatic.
    role_dropdown(&panel, MailboxRole::Sent).set_selected(3);
    pump();

    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "exactly one edit for one pick: {seen:?}");
    assert_eq!(seen[0].0, AccountId::new(1));
    assert_eq!(
        seen[0].1,
        AccountEdit::MailboxRole(MailboxRole::Sent, Some("Sent Messages".to_owned())),
        "the pane reports the folder by its server path"
    );

    window.destroy();
}

pub fn a_mapping_the_server_no_longer_has_is_shown_rather_than_dropped() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    let mut data = folders();
    data.chosen = vec![(MailboxRole::Archive, "Archivio".to_owned())];
    panel.set_account_mailboxes(vec![(AccountId::new(1), data)]);
    rows(&panel)[0].emit_activate();
    pump();

    let dropdown = role_dropdown(&panel, MailboxRole::Archive);
    let entries = entries(&dropdown);
    assert_eq!(
        entries.last().map(String::as_str),
        Some("Archivio (not on this server)"),
        "a folder that has gone is named, not silently forgotten: {entries:?}"
    );
    assert_eq!(
        dropdown.selected() as usize,
        entries.len() - 1,
        "and it is what the row shows, because it is what is stored"
    );

    window.destroy();
}

pub fn an_account_with_no_folders_yet_says_so_instead_of_offering_nothing() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    panel.set_account_mailboxes(vec![(AccountId::new(1), AccountMailboxes::default())]);
    rows(&panel)[0].emit_activate();
    pump();

    let empty = collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-mailboxes-empty",
    );
    assert_eq!(empty.len(), 1, "an empty group explains itself");
    assert!(
        collect(
            panel.upcast_ref::<gtk::Widget>(),
            "postio-settings-account-detail-role"
        )
        .is_empty(),
        "and offers no dropdown over folders nobody has yet"
    );

    window.destroy();
}
