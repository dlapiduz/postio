use crate::support_compose::pump;
use crate::support_reader::collect;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::support_compose::{fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};
use postio_widgets::settings::{AccountAction, SettingsPanel};

fn an_account(id: i64, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.id = AccountId::new(id);
    account
}

pub fn accounts_render_as_rows_and_hide_when_there_are_none() {
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

fn reindexing_in(row: &gtk::ListBoxRow) -> Option<String> {
    fact_in(row, |fact| fact.starts_with("Rebuilding"))
}

pub fn a_refused_password_offers_a_new_one_on_its_row() {
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
