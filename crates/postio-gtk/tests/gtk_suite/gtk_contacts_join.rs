//! Joining people, and acting on one of a person's addresses, from the
//! keyboard (specs/005-contacts User Story 2, FR-012..FR-016).
//!
//! The pane asks the app for what it cannot know -- the people's names, who
//! owns an address -- and acts the answer through the window, the road every
//! other gesture takes. What the tests read is what the window was asked to
//! act, and what the panel draws.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, ContactAddressAction, ContactJoinAction};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactDetail, ContactId, ContactListRow, ContactSource,
    ContactState, EmailAddress,
};

fn rows() -> Vec<ContactListRow> {
    ["Ada at work", "Ada at home", "Grace Hopper"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| ContactListRow {
            id: ContactId::new(i as i64 + 1),
            name: name.to_owned(),
            preferred: Some(format!("p{}@example.com", i + 1)),
            address_count: 1,
            last_seen_at: None,
            source: ContactSource::Mail,
            state: ContactState::Live,
            sort_key: name.to_lowercase(),
        })
        .collect()
}

/// A window with the Contacts screen open over three people, and every
/// command it is asked to act.
fn open() -> Option<(Window, Rc<RefCell<Vec<Command>>>)> {
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
    pane.connect_query({
        let pane = pane.downgrade();
        move |_, _| {
            if let Some(pane) = pane.upgrade() {
                pane.reset(3);
            }
        }
    });
    pane.connect_page({
        let pane = pane.downgrade();
        move |_, generation, page| {
            if let Some(pane) = pane.upgrade() {
                pane.deliver(generation, page, rows(), 3);
            }
        }
    });
    let acted: Rc<RefCell<Vec<Command>>> = Rc::default();
    window.connect_action({
        let acted = acted.clone();
        move |command| acted.borrow_mut().push(command)
    });
    window.act(Command::OpenContacts);
    pump();
    Some((window, acted))
}

fn joins(acted: &RefCell<Vec<Command>>) -> Vec<ContactJoinAction> {
    acted
        .borrow()
        .iter()
        .filter_map(|c| match c {
            Command::ContactJoin(action) => Some(action.clone()),
            _ => None,
        })
        .collect()
}

pub fn m_joins_the_marked_people_under_the_preselected_name() {
    let Some((window, acted)) = open() else {
        return;
    };
    let pane = window.contacts();
    let asked: Rc<RefCell<Vec<Vec<ContactId>>>> = Rc::default();
    pane.connect_join_asked({
        let asked = asked.clone();
        move |people| asked.borrow_mut().push(people)
    });

    // One person is not a join: said, and nothing asked.
    window.act(Command::ContactJoin(ContactJoinAction::Ask));
    pump();
    assert!(asked.borrow().is_empty());
    assert!(
        pane.hint_text().contains("two"),
        "hint: {:?}",
        pane.hint_text()
    );
    assert!(
        joins(&acted).iter().all(|j| *j == ContactJoinAction::Ask),
        "an Ask is the pane's to answer, never a join"
    );

    // `x`, down, `x`, `m`.
    pane.dispatch(CommandId::ToggleSelection);
    pane.set_cursor(1);
    pump();
    pane.dispatch(CommandId::ToggleSelection);
    window.act(Command::ContactJoin(ContactJoinAction::Ask));
    pump();
    assert_eq!(
        *asked.borrow(),
        [vec![ContactId::new(1), ContactId::new(2)]],
        "the app is asked for the marked people's names"
    );

    // The app answers with the choices; the panel draws them.
    pane.show_join(postio_ui::contacts::JoinChoices {
        names: vec!["Ada Lovelace".into(), "A. L.".into()],
        organizations: vec![],
        into: ContactId::new(2),
    });
    pump();
    assert!(pane.join_open());
    assert_eq!(pane.join_names(), ["Ada Lovelace", "A. L."]);

    // `Return` takes the preselected name.
    window.act(Command::ContactShowMail);
    pump();
    assert!(
        !pane.join_open(),
        "the panel goes once the join is asked for"
    );
    assert!(pane.is_open(), "and the screen stays");
    assert_eq!(
        joins(&acted).last(),
        Some(&ContactJoinAction::Join {
            into: ContactId::new(2),
            others: vec![ContactId::new(1)],
            name: "Ada Lovelace".into(),
            organization: None,
        })
    );
    window.destroy();
}

pub fn esc_cancels_a_join_and_leaves_the_screen_up() {
    let Some((window, acted)) = open() else {
        return;
    };
    let pane = window.contacts();
    pane.dispatch(CommandId::ToggleSelection);
    pane.set_cursor(2);
    pump();
    pane.dispatch(CommandId::ToggleSelection);
    pane.show_join(postio_ui::contacts::JoinChoices {
        names: vec!["Ada".into()],
        organizations: vec!["Engines Ltd".into(), "Looms & Co".into()],
        into: ContactId::new(1),
    });
    pump();
    assert_eq!(
        pane.join_organizations(),
        ["Engines Ltd", "Looms & Co"],
        "two organisations disagree, so the panel asks which to keep"
    );
    window.act(Command::Back);
    pump();
    assert!(!pane.join_open());
    assert!(pane.is_open(), "Esc took back the panel, not the screen");
    assert!(
        !joins(&acted)
            .iter()
            .any(|j| matches!(j, ContactJoinAction::Join { .. })),
        "nothing was joined"
    );
    window.destroy();
}

fn ada() -> ContactDetail {
    let address = |id: i64, email: &str| ContactAddress {
        id: AddressId::new(id),
        address: EmailAddress::new(None::<String>, email),
        times_seen: 1,
        last_seen_at: None,
        written: 0,
    };
    let mut person = Contact::new(address(10, "ada@work.example"));
    person.id = ContactId::new(1);
    person.name = Some("Ada".into());
    person.addresses.push(address(11, "ada@home.example"));
    ContactDetail {
        person,
        groups: Vec::new(),
        messages: 0,
    }
}

pub fn detach_and_preferred_act_on_the_focused_address() {
    let Some((window, acted)) = open() else {
        return;
    };
    let pane = window.contacts();
    pane.set_detail(Some(ada()));
    pump();

    // From the list, there is no address to act on.
    window.act(Command::ContactDetachAddress { address: None });
    pump();
    assert!(
        pane.hint_text().contains("address"),
        "hint: {:?}",
        pane.hint_text()
    );

    pane.focus_address(1);
    pump();
    window.act(Command::ContactDetachAddress { address: None });
    window.act(Command::ContactSetPreferred {
        person: None,
        address: None,
    });
    pump();
    let concrete: Vec<Command> = acted
        .borrow()
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::ContactDetachAddress { address: Some(_) }
                    | Command::ContactSetPreferred {
                        address: Some(_),
                        ..
                    }
            )
        })
        .cloned()
        .collect();
    assert_eq!(
        concrete,
        [
            Command::ContactDetachAddress {
                address: Some(AddressId::new(11))
            },
            Command::ContactSetPreferred {
                person: Some(ContactId::new(1)),
                address: Some(AddressId::new(11)),
            },
        ]
    );
    window.destroy();
}

pub fn plus_adds_a_typed_address_and_asks_before_taking_one() {
    let Some((window, acted)) = open() else {
        return;
    };
    let pane = window.contacts();
    pane.set_detail(Some(ada()));
    let typed: Rc<RefCell<Vec<(ContactId, String)>>> = Rc::default();
    pane.connect_add_address({
        let typed = typed.clone();
        move |person, text| typed.borrow_mut().push((person, text))
    });

    window.act(Command::ContactAddAddress(ContactAddressAction::Ask));
    pump();
    assert!(pane.address_entry_open());
    pane.submit_address("grace@example.org");
    pump();
    assert_eq!(
        *typed.borrow(),
        [(ContactId::new(1), "grace@example.org".to_owned())],
        "the app is asked to add it -- it knows who owns what"
    );

    // Someone has it: the app asks the pane to ask the user.
    pane.confirm_move(
        AddressId::new(20),
        "grace@example.org",
        "Grace Hopper",
        ContactId::new(1),
    );
    pump();
    assert!(
        pane.move_prompt()
            .is_some_and(|p| p.contains("Grace Hopper")),
        "prompt: {:?}",
        pane.move_prompt()
    );
    window.act(Command::ContactShowMail);
    pump();
    assert!(pane.move_prompt().is_none());
    assert!(acted.borrow().iter().any(|c| *c
        == Command::ContactAddAddress(ContactAddressAction::Put {
            address: AddressId::new(20),
            to: Some(ContactId::new(1)),
            revive: None,
        })));
    window.destroy();
}
