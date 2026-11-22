use crate::support_compose::pump;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::support_compose::{fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::event::MailFootprint;
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};
use postio_widgets::settings::{AccountAction, SettingsPanel};

fn an_account(id: i64, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.id = AccountId::new(id);
    account
}

pub fn accounts_render_as_rows_and_hide_when_there_are_none() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
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

    assert!(rows(&panel).is_empty(), "no accounts, no rows");
    assert!(
        !section_visible(&panel),
        "an empty accounts section should not show at all"
    );

    let mut first = an_account(1, "Ada", "ada@example.com");
    first.enabled = true;
    let mut second = an_account(2, "Grace", "grace@example.com");
    second.enabled = false;
    panel.set_accounts(vec![first, second]);
    pump();

    assert!(section_visible(&panel));
    assert_eq!(rows(&panel).len(), 2, "one row per account");

    panel.set_accounts(vec![]);
    pump();
    assert!(rows(&panel).is_empty());
    assert!(!section_visible(&panel));

    window.destroy();
}

pub fn flipping_the_switch_reports_the_account_and_the_new_state() {
    let Some((window, panel, all_rows)) = two_accounts() else {
        return;
    };

    let heard: Rc<RefCell<Vec<(AccountId, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    panel.connect_account_enabled_changed({
        let heard = heard.clone();
        move |id, enabled| heard.borrow_mut().push((id, enabled))
    });

    let switch = switch_in(&all_rows[0]);
    let before = switch.is_active();
    // A bare click cannot be synthesized on a `GtkSwitch`; `set_active` is what a drag or Space
    // does to the property.
    switch.set_active(!before);
    pump();

    assert_eq!(
        *heard.borrow(),
        vec![(AccountId::new(1), !before)],
        "the row's own id and the flipped state, not the other row's"
    );

    window.destroy();
}

pub fn the_context_menu_reaches_the_action_handler_with_the_right_account() {
    let Some((window, panel, all_rows)) = two_accounts() else {
        return;
    };

    let heard: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::new(RefCell::new(Vec::new()));
    panel.connect_account_action({
        let heard = heard.clone();
        move |id, action| heard.borrow_mut().push((id, action))
    });

    panel.test_open_account_menu(1.0, row_y(&all_rows[1]));
    assert!(
        panel
            .activate_action("account.update-credential", None)
            .is_ok()
    );
    panel.test_close_account_menu();

    panel.test_open_account_menu(1.0, row_y(&all_rows[1]));
    assert!(panel.activate_action("account.rebuild-index", None).is_ok());
    panel.test_close_account_menu();

    panel.test_open_account_menu(1.0, row_y(&all_rows[1]));
    assert!(panel.activate_action("account.remove", None).is_ok());
    panel.test_close_account_menu();

    assert_eq!(
        *heard.borrow(),
        vec![
            (AccountId::new(2), AccountAction::UpdateCredential),
            (AccountId::new(2), AccountAction::RebuildIndex),
            (AccountId::new(2), AccountAction::Remove),
        ],
        "all three should have fired for the second row, not the first"
    );

    window.destroy();
}

fn two_accounts() -> Option<(gtk::Window, SettingsPanel, Vec<gtk::ListBoxRow>)> {
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
    window.set_default_size(500, 500);
    window.present();
    pump();

    panel.set_accounts(vec![
        an_account(1, "Ada", "ada@example.com"),
        an_account(2, "Grace", "grace@example.com"),
    ]);
    pump();
    if !frames(&window, 2) {
        eprintln!("skipping: the compositor is not painting this window");
        return None;
    }

    let all_rows = rows(&panel);
    assert_eq!(all_rows.len(), 2);
    Some((window, panel, all_rows))
}

/// The weight is on the account row because it is per account and the setting is global.
pub fn an_account_row_says_what_its_mail_weighs() {
    let Some((window, panel, _)) = two_accounts() else {
        return;
    };

    // Nothing measured yet: no line at all, since `0 B` reads as a bug rather than "no mail".
    assert_eq!(weight_in(&rows(&panel)[0]), None);
    assert_eq!(weight_in(&rows(&panel)[1]), None);

    let ada = MailFootprint {
        total_bytes: 12_884_901_888,
        attachment_bytes: 11_811_160_064,
        local_bytes: 933_232_640,
        complete: true,
    };
    panel.set_mail_weights(&[(AccountId::new(1), ada)], false);
    pump();

    assert_eq!(
        weight_in(&rows(&panel)[0]).as_deref(),
        Some("890 MB downloaded · attachments would add 11 GB"),
        "a policy that is not fetching payloads owes the cost of switching"
    );
    assert_eq!(
        weight_in(&rows(&panel)[1]),
        None,
        "an account nothing has measured makes no claim"
    );
    let facts = collect(
        rows(&panel)[0].upcast_ref::<gtk::Widget>(),
        "postio-settings-account-metadata",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .expect("the row's facts line");
    assert!(
        facts.wraps() && facts.ellipsize() == gtk::pango::EllipsizeMode::None,
        "the facts line must wrap, not truncate"
    );
    panel.set_mail_weights(&[(AccountId::new(1), ada)], true);
    pump();
    assert_eq!(
        weight_in(&rows(&panel)[0]).as_deref(),
        Some("890 MB of 12 GB downloaded · attachments included")
    );

    // The weights survive a rebuild of the rows and arriving before the accounts: the two calls
    // have no order.
    panel.set_accounts(vec![
        an_account(1, "Ada", "ada@example.com"),
        an_account(2, "Grace", "grace@example.com"),
    ]);
    pump();
    assert_eq!(
        weight_in(&rows(&panel)[0]).as_deref(),
        Some("890 MB of 12 GB downloaded · attachments included"),
        "redrawing the rows must not lose what was measured for them"
    );

    window.destroy();
}

fn weight_in(row: &gtk::ListBoxRow) -> Option<String> {
    // The weight shares one mono line with the badge and other facts (#1179) and carries its own
    // `·`, so it cannot be picked out by splitting.
    let line = metadata_in(row);
    let after_badge = line.splitn(3, " · ").nth(2)?;
    let weight: Vec<&str> = after_badge
        .split(" · ")
        .take_while(|fact| {
            !fact.starts_with("token ") && !fact.starts_with("Rebuilding") && *fact != "disabled"
        })
        .collect();
    (!weight.is_empty()).then(|| weight.join(" · "))
}

fn fact_in(row: &gtk::ListBoxRow, wanted: impl Fn(&str) -> bool) -> Option<String> {
    metadata_in(row)
        .split(" · ")
        .map(str::trim)
        .find(|fact| wanted(fact))
        .map(str::to_owned)
}

fn metadata_in(row: &gtk::ListBoxRow) -> String {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-metadata",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .expect("every account row has a metadata line")
    .text()
    .to_string()
}

fn badge_in(row: &gtk::ListBoxRow) -> String {
    metadata_in(row)
        .split(" · ")
        .take(2)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn validity_in(row: &gtk::ListBoxRow) -> Option<String> {
    fact_in(row, |fact| fact.starts_with("token "))
}

fn reindexing_in(row: &gtk::ListBoxRow) -> Option<String> {
    fact_in(row, |fact| fact.starts_with("Rebuilding"))
}

pub fn an_account_row_says_how_it_connects() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();

    let mut imap_password = an_account(1, "Ada", "ada@example.com");
    imap_password.backend = postio_model::account::Backend::Imap;
    imap_password.auth = postio_model::account::AuthMethod::Password;

    let mut gmail_oauth = an_account(2, "Grace", "grace@example.com");
    gmail_oauth.backend = postio_model::account::Backend::Gmail;
    gmail_oauth.auth = postio_model::account::AuthMethod::OAuth2;

    panel.set_accounts(vec![imap_password, gmail_oauth]);
    pump();

    let all_rows = rows(&panel);
    assert_eq!(badge_in(&all_rows[0]), "IMAP · password");
    assert_eq!(badge_in(&all_rows[1]), "Gmail · OAuth 2");

    window.destroy();
}

pub fn an_account_row_says_whether_its_token_is_still_good() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();

    let password_account = an_account(1, "Ada", "ada@example.com");
    let mut oauth_account = an_account(2, "Grace", "grace@example.com");
    oauth_account.auth = postio_model::account::AuthMethod::OAuth2;
    panel.set_accounts(vec![password_account, oauth_account]);
    pump();

    let all_rows = rows(&panel);
    assert_eq!(
        validity_in(&all_rows[0]),
        None,
        "nobody told this panel about a password account's token, because it has none"
    );
    assert_eq!(
        validity_in(&all_rows[1]),
        None,
        "no expiry has been reported for the OAuth account yet either"
    );

    let far_off = std::time::SystemTime::now() + Duration::from_secs(41 * 24 * 60 * 60);
    panel.set_token_expiries(&[(AccountId::new(2), Some(far_off))]);
    pump();
    let validity = validity_in(&rows(&panel)[1]).expect("an expiry was reported");
    assert!(
        validity.starts_with("token valid 4") && validity.ends_with('d'),
        "expected roughly 41 days out, got {validity:?}"
    );

    let already_past = std::time::SystemTime::now() - Duration::from_secs(60);
    panel.set_token_expiries(&[(AccountId::new(2), Some(already_past))]);
    pump();
    assert_eq!(
        validity_in(&rows(&panel)[1]).as_deref(),
        Some("token expired — re-authorization needed")
    );

    window.destroy();
}

pub fn a_refused_password_offers_a_new_one_on_its_row() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();
    panel.set_accounts(vec![an_account(1, "Ada", "ada@example.com")]);
    pump();
    assert_eq!(
        repair_button_in(&rows(&panel)[0]),
        None,
        "nothing is wrong yet"
    );

    let heard: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::new(RefCell::new(Vec::new()));
    panel.connect_account_action({
        let heard = heard.clone();
        move |id, action| heard.borrow_mut().push((id, action))
    });
    panel.set_credential_trouble(
        AccountId::new(1),
        Some(postio_ui::account::Trouble::Rejected),
    );
    pump();

    let row = &rows(&panel)[0];
    assert_eq!(
        fact_in(row, |fact| fact.starts_with("The server")).as_deref(),
        Some("The server refused this account's password.")
    );
    let button = repair_button_in(row).expect("a refused password has a repair");
    assert_eq!(button.label().as_deref(), Some("New password…"));
    button.emit_clicked();
    assert_eq!(
        *heard.borrow(),
        vec![(AccountId::new(1), AccountAction::UpdateCredential)]
    );

    panel.set_credential_trouble(AccountId::new(1), None);
    pump();
    assert_eq!(
        repair_button_in(&rows(&panel)[0]),
        None,
        "the repair went through"
    );

    window.destroy();
}

fn repair_button_in(row: &gtk::ListBoxRow) -> Option<gtk::Button> {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-repair",
    )
    .into_iter()
    .find_map(|widget| widget.downcast::<gtk::Button>().ok())
}

pub fn an_account_row_says_when_its_search_index_is_being_rebuilt() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let panel = SettingsPanel::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&panel));
    window.present();

    panel.set_accounts(vec![
        an_account(1, "Ada", "ada@example.com"),
        an_account(2, "Grace", "grace@example.com"),
    ]);
    pump();

    assert_eq!(
        reindexing_in(&rows(&panel)[0]),
        None,
        "nothing is rebuilding either account's index yet"
    );

    panel.set_reindex_progress(AccountId::new(2), Some((0, 40)));
    pump();
    assert_eq!(
        reindexing_in(&rows(&panel)[0]),
        None,
        "the first account's row must not report the second account's rebuild"
    );
    assert_eq!(
        reindexing_in(&rows(&panel)[1]).as_deref(),
        Some("Rebuilding search index — 0 of 40")
    );

    panel.set_reindex_progress(AccountId::new(2), Some((17, 40)));
    pump();
    assert_eq!(
        reindexing_in(&rows(&panel)[1]).as_deref(),
        Some("Rebuilding search index — 17 of 40")
    );

    panel.set_reindex_progress(AccountId::new(2), None);
    pump();
    assert_eq!(
        reindexing_in(&rows(&panel)[1]),
        None,
        "the line clears once the rebuild is over"
    );

    window.destroy();
}

/// `is_mapped()` is not enough: a row can be mapped with its bounds still the pre-layout
/// placeholder.
fn frames(window: &gtk::Window, count: u32) -> bool {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
    left.get() == 0
}

fn row_y(row: &gtk::ListBoxRow) -> f64 {
    let parent = row.parent().expect("a row in a list has a parent");
    let bounds = row
        .compute_bounds(&parent)
        .expect("a mapped row has bounds");
    (bounds.y() + bounds.height() / 2.0) as f64
}

fn switch_in(row: &gtk::ListBoxRow) -> gtk::Switch {
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Switch>().ok())
        .expect("every account row has a switch")
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

fn section_visible(panel: &SettingsPanel) -> bool {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-accounts",
    )
    .into_iter()
    .any(|w| w.is_visible())
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

fn default_line_in(row: &gtk::ListBoxRow) -> Option<String> {
    let tag = collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-default",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .filter(|label| label.is_visible())?;
    let explanation = tag.tooltip_text().map(|text| text.to_string());
    Some(format!(
        "{} — {}",
        tag.text(),
        explanation.unwrap_or_default()
    ))
}

/// Words rather than colour alone (ADR 0005), and the line says what the marker does, never a
/// status like "primary" (#960).
pub fn the_default_account_says_so_and_the_others_do_not() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
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

    // Neither marked is a normal state forever; the page must not nag or pick one.
    panel.set_accounts(vec![
        an_account(1, "Ada", "ada@example.com"),
        an_account(2, "Grace", "grace@example.com"),
    ]);
    pump();
    let drawn = rows(&panel);
    assert!(
        drawn.iter().all(|row| default_line_in(row).is_none()),
        "with no default set, no row may claim to be one: choosing on the \
         user's behalf is what this issue exists to stop"
    );

    let mut ada = an_account(1, "Ada", "ada@example.com");
    ada.is_default = true;
    panel.set_accounts(vec![ada, an_account(2, "Grace", "grace@example.com")]);
    pump();
    let drawn = rows(&panel);
    let marked = default_line_in(&drawn[0]).expect("the marked account says so");
    assert!(
        marked.to_lowercase().contains("new messages"),
        "the line has to say what the marker does, not assert a status -- \
         got {marked:?}"
    );
    assert!(
        !marked.to_lowercase().contains("primary"),
        "the word is \"default\", never \"primary\": #960 recorded that as a \
         deliberate vocabulary decision. Got {marked:?}"
    );
    assert_eq!(
        default_line_in(&drawn[1]),
        None,
        "and exactly one row carries it"
    );

    // The row is announced as a unit through `account_row`; GTK exposes no reader for an accessible
    // property, so only the code building the announcement is checked.

    window.destroy();
}
