//! Adding an account to a Focus that is already running (#64): the key opens
//! a blank form over the inbox, and the inbox behind it is not disturbed.
//!
//! `first_run` holds the other host of the same form -- a store with no
//! account, where the form opens by itself. Here an account and its mail
//! are already on screen, so the form must float over the window and never
//! replace what it shows, and it must be a *new* account's form, not a
//! repair of the one already there.

use adw::prelude::*;
use postio_widgets::onboarding::{Onboarding, Status};

use crate::support;

/// The account form anywhere under `widget`.
fn form_in(widget: &gtk::Widget) -> Option<Onboarding> {
    if let Ok(form) = widget.clone().downcast::<Onboarding>() {
        return Some(form);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(form) = form_in(&current) {
            return Some(form);
        }
        child = current.next_sibling();
    }
    None
}

pub fn the_add_account_key_opens_a_blank_form_over_the_running_window() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        let mail_before = support::subjects(&window);
        assert_eq!(mail_before.len(), 3, "the inbox has its mail on screen");
        assert!(
            window.add_account_dialog().is_none(),
            "an add-account form appeared with nobody asking for one"
        );

        // The registry's binding, through the window's own resolver: the
        // road a command-bar row takes, which a test of the dialog cannot
        // prove reaches anything.
        support::deliver_with(
            &window,
            "N",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "Ctrl+Shift+N reached no handler: the add-account command resolves \
             and then hits nothing"
        );
        let dialog = window.add_account_dialog().expect("the dialog");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
        assert_eq!(
            form.address(),
            "",
            "the form arrived with an address already in it: it is a new \
             account, not a repair of the one already there"
        );
        assert!(
            matches!(form.status(), Status::Idle),
            "the form did not arrive idle: {:?}",
            form.status()
        );

        // None of it disturbed the window behind.
        assert_eq!(
            support::subjects(&window),
            mail_before,
            "opening the form changed what the running window shows: it must \
             float over the inbox, never replace it as first run's does"
        );
        assert!(
            window.pane().is_some(),
            "the inbox is no longer the window's content"
        );
    });
}

/// The sheet is as tall as the form it holds: its header is at the top of
/// the dialog, not floating in a tall empty sheet with as much blank below.
pub fn the_add_account_dialog_is_as_tall_as_its_form() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        window.act(postio_core::CommandId::AddAccount);
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "the add-account form did not open"
        );
        let dialog = window.add_account_dialog().expect("the dialog");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
        crate::settle();
        // What the form asks for at the dialog's width, and what the
        // dialog gave it.
        let (_, natural, _, _) = form.measure(gtk::Orientation::Vertical, form.width());
        // What the dialog asks for, which a fixed tall sheet answers with its
        // fixed height however short the form is.
        let (_, asked, _, _) = dialog.measure(gtk::Orientation::Vertical, 560);
        assert!(
            asked <= natural + 80,
            "the dialog asks for {asked} px around a form that needs {natural}: \
             the sheet should be as tall as what it holds"
        );
    });
}
