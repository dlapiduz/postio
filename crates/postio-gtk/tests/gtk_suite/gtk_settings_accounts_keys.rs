//! The account rows answer the keyboard (#471, ADR 0005 Q6c).
//!
//! #464 gave each row three affordances as a `gio::SimpleActionGroup`,
//! reachable by mouse and by Tab but not by the palette, the `?` sheet or a
//! bindable key. Q6c settled the shape of the fix: a `Context::Accounts`
//! scoped to the `accounts_list` widget — deliberately not a
//! `Context::Settings` spanning a panel that also holds a `GtkTextView` of
//! the literal `config.toml`, where `d` must insert a `d`.
//!
//! These assert what a person would get, not what a layer was handed: the
//! window's context after the focus actually moves, and the callback the
//! panel actually fires — the same one the context menu drives, so the
//! keyboard path and the mouse path cannot drift apart.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, Context};
use postio_gtk::settings::AccountAction;
use postio_gtk::window::Window;
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};

use crate::pump;

/// An account with a real, distinct id: `Account::new` alone leaves every
/// account sharing `AccountId::UNASSIGNED`, and "the right row" needs ids
/// that differ to mean anything.
fn an_account(id: i64, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.id = AccountId::new(id);
    account
}

/// A window with its settings panel open on two accounts.
fn ready() -> Option<(Window, AccountId)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let window = Window::default();
    window.present();
    window.open_settings();
    let second = AccountId::new(2);
    window.settings().set_accounts(vec![
        an_account(1, "ada", "ada@example.com"),
        an_account(2, "quinn", "quinn@example.test"),
    ]);
    pump();
    Some((window, second))
}

pub fn focus_on_an_account_row_enters_the_accounts_context_and_leaving_restores_it() {
    let Some((window, _)) = ready() else { return };

    window.set_context(Context::List);
    let list = window.settings().accounts_list();
    let row = list.row_at_index(0).expect("a first account row");
    row.grab_focus();
    pump();

    assert_eq!(
        window.context(),
        Context::Accounts,
        "the context must follow the keyboard into the account list, or `d` \
         means delete-message while the focus ring sits on an account"
    );

    // Somewhere else in the window, the way Tab out of the list would.
    window.list().grab_focus();
    pump();
    assert_eq!(
        window.context(),
        Context::List,
        "leaving the account list must restore the context it interrupted, \
         not strand the window in Accounts where `a` no longer archives"
    );
}

pub fn remove_account_acts_on_the_row_the_keyboard_is_on() {
    let Some((window, second)) = ready() else {
        return;
    };

    let seen: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::default();
    window.settings().connect_account_action({
        let seen = Rc::clone(&seen);
        move |id, action| seen.borrow_mut().push((id, action))
    });

    // The keyboard on the *second* row, so a handler that ignored focus and
    // took the first account would be caught.
    let list = window.settings().accounts_list();
    list.row_at_index(1)
        .expect("a second account row")
        .grab_focus();
    pump();

    window.act(Command::RemoveAccount);
    pump();

    assert_eq!(
        seen.borrow().as_slice(),
        [(second, AccountAction::Remove)],
        "the command must fire the same callback the context menu fires, for \
         the focused row — anything else is a second, divergent path to an \
         account-scale deletion"
    );
}

pub fn update_credential_acts_on_the_row_the_keyboard_is_on() {
    let Some((window, second)) = ready() else {
        return;
    };

    let seen: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::default();
    window.settings().connect_account_action({
        let seen = Rc::clone(&seen);
        move |id, action| seen.borrow_mut().push((id, action))
    });

    let list = window.settings().accounts_list();
    list.row_at_index(1)
        .expect("a second account row")
        .grab_focus();
    pump();

    window.act(Command::UpdateCredential);
    pump();

    assert_eq!(
        seen.borrow().as_slice(),
        [(second, AccountAction::UpdateCredential)]
    );
}

pub fn toggling_enabled_flips_the_focused_rows_switch_and_reports_it() {
    let Some((window, second)) = ready() else {
        return;
    };

    let seen: Rc<RefCell<Vec<(AccountId, bool)>>> = Rc::default();
    window.settings().connect_account_enabled_changed({
        let seen = Rc::clone(&seen);
        move |id, enabled| seen.borrow_mut().push((id, enabled))
    });

    let list = window.settings().accounts_list();
    list.row_at_index(1)
        .expect("a second account row")
        .grab_focus();
    pump();

    window.act(Command::ToggleAccountEnabled);
    pump();

    assert_eq!(
        seen.borrow().as_slice(),
        [(second, false)],
        "the command must move the switch the person can see, so that the \
         row and the stored column cannot disagree — an account is enabled \
         by default, so one press disables it"
    );

    // Pressing it again is the reversal; that is why the registry gives it
    // `Recovery::None` rather than an undo entry.
    window.act(Command::ToggleAccountEnabled);
    pump();
    assert_eq!(seen.borrow().len(), 2);
    assert_eq!(seen.borrow()[1], (second, true));
}

pub fn the_account_commands_do_nothing_when_the_keyboard_is_elsewhere() {
    let Some((window, _)) = ready() else { return };

    let seen: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::default();
    window.settings().connect_account_action({
        let seen = Rc::clone(&seen);
        move |id, action| seen.borrow_mut().push((id, action))
    });

    // No row focused, so no target. The palette can offer these while the
    // context is live, and a command that fell back to "the first account"
    // would remove somebody's mail on a keystroke aimed at nothing.
    window.list().grab_focus();
    window.set_context(Context::Accounts);
    pump();

    window.act(Command::RemoveAccount);
    pump();

    assert!(
        seen.borrow().is_empty(),
        "with no account row focused there is no target, and guessing one is \
         how a keystroke aimed at nothing removes an account"
    );
}

pub fn undo_in_the_account_list_reaches_the_removal_toast() {
    let Some((window, _)) = ready() else { return };

    // #464 wired removal's undo straight to `AccountRepository::restore`
    // rather than through the global stack, and said so because Remove was
    // not a command then. Registering it with `Recovery::Undo` makes that a
    // declaration the registry enforces — and a declaration nothing backs
    // from the keyboard is what ADR 0005 keeps refusing to ship.
    let undone = Rc::new(std::cell::Cell::new(0));
    window.show_removable_toast("Removed quinn", {
        let undone = Rc::clone(&undone);
        move || undone.set(undone.get() + 1)
    });
    window.settings().accounts_list().grab_focus();
    window.set_context(Context::Accounts);
    pump();

    window.act(Command::Undo);
    pump();

    assert_eq!(
        undone.get(),
        1,
        "`u` in the account list must reach the toast's own undo; the global \
         stack never held this removal, so nothing else can"
    );
}

pub fn undo_outside_the_account_list_leaves_the_removal_toast_alone() {
    let Some((window, _)) = ready() else { return };

    let undone = Rc::new(std::cell::Cell::new(0));
    window.show_removable_toast("Removed quinn", {
        let undone = Rc::clone(&undone);
        move || undone.set(undone.get() + 1)
    });
    window.set_context(Context::List);
    pump();

    window.act(Command::Undo);
    pump();

    assert_eq!(
        undone.get(),
        0,
        "`u` over the message list means the global undo stack. If it also \
         restored an account whose toast happened to be up, one keystroke \
         would quietly do two things"
    );
}

/// The fifth account row answers `m`, on the row the keyboard is on (#960).
///
/// The same seam the context menu drives, so the two paths cannot drift: a
/// keyboard binding that reached a different callback would be a second
/// implementation of "which account is the default", which is what the
/// registry exists to prevent.
pub fn set_default_account_acts_on_the_row_the_keyboard_is_on() {
    let Some((window, second)) = ready() else {
        return;
    };

    let seen: Rc<RefCell<Vec<(AccountId, AccountAction)>>> = Rc::default();
    window.settings().connect_account_action({
        let seen = Rc::clone(&seen);
        move |id, action| seen.borrow_mut().push((id, action))
    });

    let list = window.settings().accounts_list();
    list.row_at_index(1)
        .expect("a second account row")
        .grab_focus();
    pump();

    window.act(Command::SetDefaultAccount);
    pump();

    assert_eq!(
        seen.borrow().as_slice(),
        [(second, AccountAction::SetDefault)],
        "`m` marks the focused row, not the first one and not all of them"
    );
}
