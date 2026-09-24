//! Possible duplicates, from the keyboard (specs/005-contacts User Story 4,
//! FR-018, FR-019): `v s` shows them with their evidence, `m` joins the
//! focused pair through the join panel, and `X` dismisses it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, ContactJoinAction};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactId, EmailAddress, JoinSuggestion, SuggestionReason,
};

fn person(id: i64, name: &str, email: &str, seen: u32) -> Contact {
    let mut person = Contact::new(ContactAddress {
        id: AddressId::new(id * 10),
        address: EmailAddress::new(None::<String>, email),
        times_seen: seen,
        last_seen_at: None,
        written: 0,
    });
    person.id = ContactId::new(id);
    person.seen_name = Some(name.into());
    person
}

fn suggestion() -> JoinSuggestion {
    JoinSuggestion {
        people: [
            person(1, "Ada Lovelace", "ada@work.example", 3),
            person(2, "Ada Lovelace", "ada@home.example", 1),
        ],
        reason: SuggestionReason::SameName,
    }
}

pub fn v_s_shows_the_evidence_m_joins_and_x_dismisses() {
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
                pane.reset(0);
            }
        }
    });
    let asked: Rc<RefCell<u32>> = Rc::default();
    pane.connect_suggestions_asked({
        let asked = asked.clone();
        let pane = pane.downgrade();
        move || {
            *asked.borrow_mut() += 1;
            if let Some(pane) = pane.upgrade() {
                pane.show_suggestions(vec![suggestion()]);
            }
        }
    });
    let joins: Rc<RefCell<Vec<Vec<ContactId>>>> = Rc::default();
    pane.connect_join_asked({
        let joins = joins.clone();
        move |people| joins.borrow_mut().push(people)
    });
    let acted: Rc<RefCell<Vec<Command>>> = Rc::default();
    window.connect_action({
        let acted = acted.clone();
        move |command| acted.borrow_mut().push(command)
    });
    window.act(Command::OpenContacts);
    pump();

    pane.dispatch(CommandId::ContactsSuggestions);
    pump();
    assert_eq!(*asked.borrow(), 1);
    assert!(pane.suggestions_open());
    let lines = pane.suggestion_lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("ada@work.example") && lines[0].contains("ada@home.example"),
        "both addresses are the evidence: {lines:?}"
    );
    assert!(lines[0].contains("same name"), "and why: {lines:?}");

    // Show mail has no one person to show here.
    pane.dispatch(CommandId::ContactShowMail);
    assert!(
        pane.hint_text().contains("one of the two"),
        "hint: {:?}",
        pane.hint_text()
    );

    window.act(Command::ContactJoin(ContactJoinAction::Ask));
    pump();
    assert_eq!(
        *joins.borrow(),
        [vec![ContactId::new(1), ContactId::new(2)]]
    );

    window.act(Command::SuggestionDismiss { pair: None });
    pump();
    assert!(acted.borrow().contains(&Command::SuggestionDismiss {
        pair: Some((ContactId::new(1), ContactId::new(2)))
    }));

    pane.dispatch(CommandId::ContactsSuggestions);
    pump();
    assert!(
        !pane.suggestions_open(),
        "v s again goes back to the people"
    );
    window.destroy();
}
