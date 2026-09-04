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
use postio_gtk::settings::{AccountEdit, SettingsPanel};
use postio_gtk::{fonts, style};
use postio_model::account::{ServerConfig, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};

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

/// An account with two named signatures, one of them already the default.
fn an_account_with_signatures(id: i64) -> Account {
    let mut account = an_account(id, "Ada", "ada@example.com");
    let mut work = postio_model::Signature::new("Work", "-- \nAda, Analytical Engines");
    work.id = postio_model::ids::SignatureId::new(7);
    let mut brief = postio_model::Signature::new("Brief", "-- \nAda");
    brief.id = postio_model::ids::SignatureId::new(8);
    account.default_signature_id = Some(work.id);
    account.signatures = vec![work, brief];
    account
}

/// #979: the row lists the account's signatures and starts on its default.
///
/// A dropdown over what the account has, not a path field: `Account` carries
/// `signatures: Vec<Signature>` and `default_signature_id`, and there has
/// never been a filesystem path for #880's mockup to have meant.
pub fn the_detail_view_offers_the_accounts_signatures_and_starts_on_its_default() {
    let Some((window, panel)) = new_panel() else {
        return;
    };
    panel.set_accounts(vec![an_account_with_signatures(1)]);
    pump();
    panel.open_account_detail(AccountId::new(1));
    pump();

    let picker = signature_picker(&panel);
    assert!(
        picker.is_visible(),
        "an account with signatures has something to choose between"
    );
    let names: Vec<String> = (0..picker.model().expect("a model").n_items())
        .filter_map(|n| {
            picker
                .model()
                .expect("a model")
                .item(n)
                .and_then(|o| o.downcast::<gtk::StringObject>().ok())
                .map(|s| s.string().to_string())
        })
        .collect();
    assert_eq!(
        names,
        vec!["Work".to_owned(), "Brief".to_owned()],
        "the picker lists the account's signatures by name, in its own order"
    );
    assert_eq!(
        picker.selected(),
        0,
        "and opens on the one the account already calls its default"
    );
    drop(window);
}

/// The empty state, decided by the precedent this codebase already set.
///
/// `composer.rs::set_signatures` hides its picker when the account has none
/// — *"a picker with one entry is a control that can only ever say what is
/// already true"* — and `set_accounts` cites that rule for hiding an empty
/// section. The same answer applies here for a sharper reason: **nothing in
/// Postio creates a signature yet.** `Signature::new` has no caller outside
/// seeds and tests, so a row prompting "add one" would point at a flow that
/// does not exist, which is the failure `/issue` §4 is about.
pub fn an_account_with_no_signatures_gets_no_picker_at_all() {
    let Some((window, panel)) = panel_with_account() else {
        return;
    };
    panel.open_account_detail(AccountId::new(1));
    pump();

    assert!(
        !signature_picker(&panel).is_visible(),
        "an account with no signatures is offered a control that can only \
         ever say what is already true"
    );
    drop(window);
}

/// Choosing one reports the account and the signature, the same way every
/// other field in this view does — the panel never writes, because an
/// account is database state and `settings_accounts.rs` owns the write.
pub fn choosing_a_signature_reports_the_account_and_the_choice() {
    let Some((window, panel)) = new_panel() else {
        return;
    };
    panel.set_accounts(vec![an_account_with_signatures(1)]);
    pump();
    panel.open_account_detail(AccountId::new(1));
    pump();

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorder = std::rc::Rc::clone(&seen);
    panel.connect_account_edited(move |id, edit| recorder.borrow_mut().push((id, edit)));

    // The second entry: "Brief", which is not the current default.
    signature_picker(&panel).set_selected(1);
    pump();

    assert_eq!(
        *seen.borrow(),
        vec![(
            AccountId::new(1),
            AccountEdit::DefaultSignature(Some(postio_model::ids::SignatureId::new(8)))
        )],
        "one edit naming the account and the signature it chose"
    );
    drop(window);
}

/// Opening an account must not look like editing it.
///
/// Filling the picker moves its selection, and a `DropDown` cannot tell a
/// programmatic move from a person's — so without
/// `account_detail_loading`, merely *viewing* an account would report a
/// `DefaultSignature` edit and rewrite what it was showing. The panel has
/// that guard already; this is the first test that holds a control to it,
/// because every other one connects its handler after opening and so could
/// not see the edit if it happened.
pub fn opening_an_account_reports_no_edit_of_its_own() {
    let Some((window, panel)) = new_panel() else {
        return;
    };
    // The default is the *second* signature on purpose. With the first,
    // populating sets the selection to an index it already holds, GTK emits
    // no notification, and this test passes whether or not the guard is
    // there — which is a test that cannot fail.
    let mut account = an_account_with_signatures(1);
    account.default_signature_id = Some(account.signatures[1].id);
    panel.set_accounts(vec![account]);
    pump();

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorder = std::rc::Rc::clone(&seen);
    panel.connect_account_edited(move |id, edit| recorder.borrow_mut().push((id, edit)));

    panel.open_account_detail(AccountId::new(1));
    pump();

    assert!(
        seen.borrow().is_empty(),
        "opening the view reported an edit, so viewing an account rewrites \
         it: {:?}",
        seen.borrow()
    );
    drop(window);
}

fn signature_picker(panel: &SettingsPanel) -> gtk::DropDown {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-signature",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::DropDown>().ok())
    .expect("the detail view has a signature picker")
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
