//! Groups on the Contacts screen (specs/005-contacts User Story 5, FR-040):
//! listed above the people, a group under the keyboard shows its members,
//! and every edit is a key that acts through the window.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, ContactGroupNewAction};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{
    ContactGroup, ContactGroupId, ContactId, ContactListRow, ContactSource, ContactState,
};

fn row(id: i64, name: &str) -> ContactListRow {
    ContactListRow {
        id: ContactId::new(id),
        name: name.into(),
        preferred: Some(format!("p{id}@example.com")),
        address_count: 1,
        last_seen_at: None,
        source: ContactSource::User,
        state: ContactState::Live,
        sort_key: name.to_lowercase(),
    }
}

fn group(id: i64, name: &str) -> ContactGroup {
    let mut group = ContactGroup::new(name, chrono::Utc::now());
    group.id = ContactGroupId::new(id);
    group
}

type Acted = Rc<RefCell<Vec<Command>>>;

pub fn groups_are_made_filled_renamed_emptied_and_deleted_by_key() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    let window = Window::default();
    window.present();
    pump();
    let pane = window.contacts();
    pane.connect_query({
        let pane = pane.downgrade();
        move |_, _| {
            if let Some(pane) = pane.upgrade() {
                pane.reset(2);
            }
        }
    });
    pane.connect_page({
        let pane = pane.downgrade();
        move |_, generation, page| {
            if let Some(pane) = pane.upgrade() {
                pane.deliver(generation, page, vec![row(1, "Ada"), row(2, "Grace")], 2);
            }
        }
    });
    pane.connect_groups_asked({
        let pane = pane.downgrade();
        move || {
            if let Some(pane) = pane.upgrade() {
                pane.show_groups(vec![group(7, "Family"), group(8, "Work")]);
            }
        }
    });
    let shown: Rc<RefCell<Vec<ContactGroupId>>> = Rc::default();
    pane.connect_group_rows({
        let shown = shown.clone();
        let pane = pane.downgrade();
        move |group| {
            shown.borrow_mut().push(group);
            if let Some(pane) = pane.upgrade() {
                pane.show_rows(vec![row(1, "Ada")]);
            }
        }
    });
    let mailed: Rc<RefCell<Vec<String>>> = Rc::default();
    pane.connect_group_mail({
        let mailed = mailed.clone();
        move |name| mailed.borrow_mut().push(name)
    });
    let acted: Acted = Rc::default();
    window.connect_action({
        let acted = acted.clone();
        move |command| acted.borrow_mut().push(command)
    });
    window.act(Command::OpenContacts);
    pump();
    assert_eq!(
        pane.group_lines(),
        ["Family", "Work"],
        "listed above the people"
    );

    // g n: a group of the marked people.
    pane.dispatch(CommandId::ToggleSelection);
    window.act(Command::ContactGroupNew(ContactGroupNewAction::Ask));
    pump();
    assert!(pane.group_panel_open());
    pane.submit_group_name("Book club");
    pump();
    assert!(
        acted
            .borrow()
            .contains(&Command::ContactGroupNew(ContactGroupNewAction::Create {
                name: "Book club".into(),
                members: vec![ContactId::new(1)],
            }))
    );

    // l: into a group chosen from the list, the person under the cursor.
    window.act(Command::ContactGroupAdd {
        group: None,
        people: Vec::new(),
    });
    pump();
    assert!(pane.group_panel_open(), "l asks which group");
    pane.choose_group(1);
    pump();
    assert!(acted.borrow().contains(&Command::ContactGroupAdd {
        group: Some(ContactGroupId::new(8)),
        people: vec![ContactId::new(1)],
    }));

    // The keyboard on a group shows its members.
    pane.focus_group(0);
    pump();
    assert_eq!(*shown.borrow(), [ContactGroupId::new(7)]);

    // Return shows its mail; R renames; d deletes it.
    pane.dispatch(CommandId::ContactShowMail);
    assert_eq!(*mailed.borrow(), ["Family"]);
    window.act(Command::ContactGroupRename {
        group: None,
        name: None,
    });
    pump();
    assert!(pane.group_panel_open());
    pane.submit_group_name("Kin");
    pump();
    assert!(acted.borrow().contains(&Command::ContactGroupRename {
        group: Some(ContactGroupId::new(7)),
        name: Some("Kin".into()),
    }));
    pane.focus_group(0);
    window.act(Command::ContactDelete {
        person: None,
        group: None,
    });
    pump();
    assert!(acted.borrow().contains(&Command::ContactDelete {
        person: None,
        group: Some(ContactGroupId::new(7)),
    }));

    // L: out of the group on screen.
    pane.focus_group(0);
    pump();
    pane.set_cursor(0);
    window.act(Command::ContactGroupRemove {
        group: None,
        people: Vec::new(),
    });
    pump();
    assert!(acted.borrow().contains(&Command::ContactGroupRemove {
        group: Some(ContactGroupId::new(7)),
        people: vec![ContactId::new(1)],
    }));
    window.destroy();
}
