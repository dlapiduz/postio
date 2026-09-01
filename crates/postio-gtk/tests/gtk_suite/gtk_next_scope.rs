//! `g a` cycles the sidebar's account scope, from the keyboard (#765).
//!
//! `NextScope` was a registry command — binding `g a`, doc comment "move to
//! the next account scope: unified, then each account in turn" — with no
//! handler anywhere: not in `Window::handled_here`, not on the bus.
//! Invoking it did nothing. `gtk_sidebar_accounts.rs` already proves the
//! strip reports a *click* on one of its own rows; this proves the real
//! command reaches the strip and walks its rows the same way a click would,
//! rather than only checking it reaches `Window::connect_command` — the gap
//! that let this and #756 both through unnoticed.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::AccountScope;
use postio_model::ids::AccountId;

fn pump() {
    while glib::MainContext::default().iteration(false) {}
}

pub fn g_a_cycles_the_strip_the_same_way_clicking_its_rows_does() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    let work = AccountId::new(1);
    let home = AccountId::new(2);
    let accounts = [(work, "Work".to_owned()), (home, "Home".to_owned())];
    window
        .sidebar()
        .set_accounts(&accounts, AccountScope::Unified, true);

    let picked: Rc<RefCell<Vec<AccountScope>>> = Rc::new(RefCell::new(Vec::new()));
    window.sidebar().connect_scope_selected({
        let picked = Rc::clone(&picked);
        move |scope| picked.borrow_mut().push(scope)
    });

    // ── the real command, not a widget call ────────────────────────────
    window.act(postio_core::Command::NextScope);
    pump();
    assert_eq!(
        *picked.borrow(),
        vec![AccountScope::Account(work)],
        "g a from Unified should have picked the first account, the same \
         row a click on it would have"
    );

    window.act(postio_core::Command::NextScope);
    pump();
    assert_eq!(
        *picked.borrow(),
        vec![AccountScope::Account(work), AccountScope::Account(home)],
    );

    // ── past the last account, it wraps back to the top ────────────────
    window.act(postio_core::Command::NextScope);
    pump();
    assert_eq!(
        *picked.borrow(),
        vec![
            AccountScope::Account(work),
            AccountScope::Account(home),
            AccountScope::Unified,
        ],
        "past the last account, g a should wrap back to Unified"
    );

    window.destroy();
}
