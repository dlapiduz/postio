//! The sidebar's accounts strip: Unified, then one row per account. #185.
//!
//! ADR 0005 Q4 puts Unified at the top of the sidebar as its own root, with
//! the accounts under it. This is that strip — the surface that makes a scope
//! something you can *see* and click, next to `g a`, which is the same move
//! from the keyboard.
//!
//! The sharpest requirement here is the one about people who have not
//! configured a second account: they must see no trace of any of this. It is
//! tested as absence rather than as a disabled control, because a disabled
//! control is still a question asked.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Without a display it skips. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use postio_gtk::sidebar::Sidebar;
use postio_gtk::{app, fonts, style};
use postio_model::AccountScope;
use postio_model::ids::AccountId;

pub fn the_strip_names_every_account_and_is_absent_with_one() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let sidebar = Sidebar::new();
    let work = AccountId::new(1);
    let home = AccountId::new(2);

    // ── one account: nothing at all ─────────────────────────────────────
    sidebar.set_accounts(&[(work, "Work".into())], AccountScope::Unified, true);
    assert!(
        sidebar.account_rows().is_empty(),
        "the strip is drawn for somebody with one account, where it can only \
         offer a choice between the same mail and the same mail"
    );

    // ── two: Unified first, then the accounts in the order given ────────
    let accounts = [(work, "Work".to_owned()), (home, "Home".to_owned())];
    sidebar.set_accounts(&accounts, AccountScope::Unified, true);
    assert_eq!(
        sidebar.account_rows(),
        vec!["Unified", "Work", "Home"],
        "Unified is the root and the accounts keep the caller's order — which \
         is the order their hues are keyed to"
    );

    // ── picking one reports it, exactly once ────────────────────────────
    let picked: Rc<RefCell<Vec<AccountScope>>> = Rc::new(RefCell::new(Vec::new()));
    sidebar.connect_scope_selected({
        let picked = Rc::clone(&picked);
        move |scope| picked.borrow_mut().push(scope)
    });

    sidebar.set_accounts(&accounts, AccountScope::Account(home), true);
    assert!(
        picked.borrow().is_empty(),
        "restoring the current scope is not the user picking it — a sidebar \
         that reports its own repaint puts the application in a loop"
    );

    sidebar.set_scope(AccountScope::Account(work));
    assert!(
        picked.borrow().is_empty(),
        "and neither is being told what the scope now is"
    );

    // The pointer's path, which is the one that must report.
    sidebar.test_click_account_row(2);
    assert_eq!(
        *picked.borrow(),
        vec![AccountScope::Account(home)],
        "clicking an account row is what asks for that scope"
    );

    sidebar.test_click_account_row(0);
    assert_eq!(
        *picked.borrow(),
        vec![AccountScope::Account(home), AccountScope::Unified],
        "and the top row asks for every account at once"
    );

    // ── without a unified list to show, the row is not offered ──────────
    //
    // A row that selects a scope nothing can draw is a dead end, and the
    // application is what knows whether #184 has landed. The strip is still
    // where it will appear.
    sidebar.set_accounts(&accounts, AccountScope::Account(work), false);
    assert_eq!(
        sidebar.account_rows(),
        vec!["Work", "Home"],
        "Unified is offered only when picking it leads somewhere"
    );
}
