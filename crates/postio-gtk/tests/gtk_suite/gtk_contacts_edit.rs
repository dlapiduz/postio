//! Making, editing, deleting and restoring people from the keyboard
//! (specs/005-contacts User Story 3, FR-020..FR-023a).
//!
//! The pane turns a key into the command with its person filled in, or
//! opens the editor and turns what was typed into one -- and refuses, with
//! the reason on the line, an address that does not parse. What the tests
//! read is what the window was asked to act.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, ContactEditAction, ContactNewAction};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactDetail, ContactId, ContactListRow, ContactSource,
    ContactState, ContactView, EmailAddress, PersonEdit,
};

fn rows(state: ContactState) -> Vec<ContactListRow> {
    vec![ContactListRow {
        id: ContactId::new(1),
        name: "Ada".into(),
        preferred: Some("ada@example.com".into()),
        address_count: 1,
        last_seen_at: None,
        source: ContactSource::User,
        state,
        sort_key: "ada".into(),
    }]
}

type Views = Rc<RefCell<Vec<ContactView>>>;
type Acted = Rc<RefCell<Vec<Command>>>;

fn open() -> Option<(Window, Acted, Views)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = Window::default();
    window.present();
    pump();
    let pane = window.contacts();
    let views: Views = Rc::default();
    pane.connect_query({
        let pane = pane.downgrade();
        let views = views.clone();
        move |view, _| {
            views.borrow_mut().push(view);
            if let Some(pane) = pane.upgrade() {
                pane.reset(1);
            }
        }
    });
    pane.connect_page({
        let pane = pane.downgrade();
        move |view, generation, page| {
            let state = if view == ContactView::Deleted {
                ContactState::Deleted
            } else {
                ContactState::Live
            };
            if let Some(pane) = pane.upgrade() {
                pane.deliver(generation, page, rows(state), 1);
            }
        }
    });
    let acted: Acted = Rc::default();
    window.connect_action({
        let acted = acted.clone();
        move |command| acted.borrow_mut().push(command)
    });
    window.act(Command::OpenContacts);
    pump();
    Some((window, acted, views))
}

fn ada() -> ContactDetail {
    let mut person = Contact::new(ContactAddress {
        id: AddressId::new(10),
        address: EmailAddress::new(None::<String>, "ada@example.com"),
        times_seen: 1,
        last_seen_at: None,
        written: 0,
    });
    person.id = ContactId::new(1);
    person.name = Some("Ada".into());
    person.note = Some("met at the conference".into());
    ContactDetail {
        person,
        groups: Vec::new(),
        messages: 0,
    }
}

pub fn d_deletes_the_person_under_the_cursor_and_r_restores_in_the_deleted_view() {
    let Some((window, acted, views)) = open() else {
        return;
    };
    let pane = window.contacts();

    window.act(Command::ContactDelete { person: None });
    pump();
    assert!(acted.borrow().contains(&Command::ContactDelete {
        person: Some(ContactId::new(1))
    }));

    // `r` outside the Deleted view has nobody to restore.
    window.act(Command::ContactRestore {
        person: None,
        state: None,
    });
    pump();
    assert!(
        pane.hint_text().contains("Deleted"),
        "hint: {:?}",
        pane.hint_text()
    );

    pane.dispatch(CommandId::ContactsToggleDeleted);
    pump();
    assert_eq!(views.borrow().last(), Some(&ContactView::Deleted));
    window.act(Command::ContactRestore {
        person: None,
        state: None,
    });
    pump();
    assert!(acted.borrow().contains(&Command::ContactRestore {
        person: Some(ContactId::new(1)),
        state: None,
    }));
    pane.dispatch(CommandId::ContactsToggleDeleted);
    pump();
    assert_eq!(
        views.borrow().last(),
        Some(&ContactView::Written),
        "and back"
    );
    window.destroy();
}

pub fn n_makes_a_person_from_what_was_typed_and_refuses_a_bad_address() {
    let Some((window, acted, _)) = open() else {
        return;
    };
    let pane = window.contacts();
    window.act(Command::ContactNew(ContactNewAction::Ask));
    pump();
    assert!(pane.editor_open());
    let editor = pane.editor();

    editor.name_entry().set_text("Grace Hopper");
    editor.address_entry().set_text("not an address");
    pane.save_editor();
    pump();
    assert!(pane.editor_open(), "a bad address keeps the editor up");
    assert!(
        !editor.error_text().is_empty(),
        "with the reason on the line"
    );

    editor.address_entry().set_text("grace@example.org");
    pane.save_editor();
    pump();
    assert!(!pane.editor_open());
    assert!(
        acted
            .borrow()
            .contains(&Command::ContactNew(ContactNewAction::Create {
                name: Some("Grace Hopper".into()),
                addresses: vec![EmailAddress::new(None::<String>, "grace@example.org")],
            }))
    );
    window.destroy();
}

pub fn e_edits_the_person_in_the_detail() {
    let Some((window, acted, _)) = open() else {
        return;
    };
    let pane = window.contacts();
    pane.set_detail(Some(ada()));
    window.act(Command::ContactEdit(ContactEditAction::Ask));
    pump();
    assert!(pane.editor_open());
    let editor = pane.editor();
    assert_eq!(editor.name_entry().text(), "Ada", "filled from the person");
    assert!(
        !WidgetExt::is_visible(editor.address_entry()),
        "addresses have their own keys"
    );
    editor.organization_entry().set_text("Analytical Engines");
    pane.save_editor();
    pump();
    assert!(
        acted
            .borrow()
            .contains(&Command::ContactEdit(ContactEditAction::Edit {
                person: ContactId::new(1),
                edit: PersonEdit {
                    name: Some("Ada".into()),
                    organization: Some("Analytical Engines".into()),
                    note: Some("met at the conference".into()),
                },
            }))
    );
    window.destroy();
}
