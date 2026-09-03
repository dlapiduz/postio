//! Accounts in the settings panel, on a real display (#464, ADR 0005 Q6a).
//!
//! `set_accounts` draws one row per account with an enabled switch; the
//! row's context menu is `SavedSearchAction`'s exact shape from #292 --
//! `gio::SimpleActionGroup`, addressed by name, not a `CommandId`.
//!
//! That is still how the *menu* is built, but the verbs behind it are
//! commands now: ADR 0005 Q6c added `Context::Accounts` and #471 registered
//! the three, so the keyboard reaches them through the focused row. Both
//! paths end in the same callbacks -- `gtk_settings_accounts_keys.rs` is the
//! keyboard half. Skips without a display. Nothing here touches the network.

use crate::pump;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::event::MailFootprint;
use postio_gtk::settings::{AccountAction, SettingsPanel};
use postio_gtk::{fonts, style};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};

/// An in-memory account with a real, distinct id -- `Account::new` alone
/// always gives `AccountId::UNASSIGNED`, which every unpersisted account
/// would then share, and a test asserting "the right row" needs ids that
/// actually differ to mean anything.
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
    // A bare click cannot be synthesized on a `GtkSwitch` in a test any more
    // than a `GtkPopoverMenu` item's can (see `gtk_sidebar_saved_searches.rs`
    // for the same limit hit on that widget); `set_active` is what a real
    // drag or a Space press ultimately does to the property this listens to.
    switch.set_active(!before);
    pump();

    assert_eq!(
        *heard.borrow(),
        vec![(AccountId::new(1), !before)],
        "the row's own id and the flipped state, not the other row's"
    );

    window.destroy();
}

/// #464: the context menu's two verbs reach `connect_account_action` with
/// the right account id.
///
/// Driven with [`SettingsPanel::test_open_account_menu`] and
/// `WidgetExt::activate_action`, not a synthesized click -- see that
/// method's own doc, and `Sidebar::test_open_saved_search_menu`'s, for why.
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
    assert!(panel.activate_action("account.remove", None).is_ok());
    panel.test_close_account_menu();

    assert_eq!(
        *heard.borrow(),
        vec![
            (AccountId::new(2), AccountAction::UpdateCredential),
            (AccountId::new(2), AccountAction::Remove),
        ],
        "both should have fired for the second row, not the first"
    );

    window.destroy();
}

/// A window and panel with two accounts, and every row's own
/// `gtk::ListBoxRow`. `None` means "skip": no display, or the compositor
/// never painted the frames the row geometry depends on.
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

/// Issue #411's real payoff: `attachment_fetch = "eager"` is an abstraction
/// and `attachments would add 11 GB` is a decision somebody can make.
///
/// The number lives on the account row rather than beside the setting
/// because it is per account and the setting is global -- there is no row to
/// read a summed figure off, and the panel is a `TextView` over literal TOML
/// on purpose, so a form control here would fight what it is for.
pub fn an_account_row_says_what_its_mail_weighs() {
    let Some((window, panel, _)) = two_accounts() else {
        return;
    };

    // Nothing measured yet: no line at all. `0 B` reads as a bug rather than
    // as "no mail", the same rule the status line follows.
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
    // Switching the policy re-reads the same footprint and asks the other
    // question of it.
    panel.set_mail_weights(&[(AccountId::new(1), ada)], true);
    pump();
    assert_eq!(
        weight_in(&rows(&panel)[0]).as_deref(),
        Some("890 MB of 12 GB downloaded · attachments included")
    );

    // The weights outlive a rebuild of the rows, and survive arriving before
    // the accounts do -- the two calls come from different places and there
    // is no order to rely on.
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

/// The row's own weight line, if it has one.
fn weight_in(row: &gtk::ListBoxRow) -> Option<String> {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-weight",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .filter(|label| label.is_visible())
    .map(|label| label.text().to_string())
}

/// Run the main loop until `window` has actually painted `count` frames.
///
/// `is_mapped()` becoming true is not enough -- a row can be mapped with its
/// bounds still the pre-layout placeholder. Copied from
/// `gtk_sidebar_saved_searches.rs::frames` rather than shared, matching that
/// file's own reason: no dependency between the two.
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
    let deadline = Instant::now() + Duration::from_secs(5);
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

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first.
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
