use adw::prelude::*;
use postio_widgets::onboarding::{Onboarding, Status};

use crate::support;

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
        let (_, natural, _, _) = form.measure(gtk::Orientation::Vertical, form.width());
        let (_, asked, _, _) = dialog.measure(gtk::Orientation::Vertical, 560);
        assert!(
            asked <= natural + 80,
            "the dialog asks for {asked} px around a form that needs {natural}: \
             the sheet should be as tall as what it holds"
        );
    });
}
