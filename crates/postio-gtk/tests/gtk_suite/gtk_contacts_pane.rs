//! The Contacts screen takes the reading pane and gives it back
//! (specs/005-contacts FR-001, R10).
//!
//! The contract is the composer's: the shell's one owner shows it, the
//! keyboard is in `Context::Contacts` while it is up, and `Esc` returns the
//! pane to what had it -- computed, not replayed -- and the keyboard to the
//! context and pane it came from. Rows arrive through the pane's page
//! request, answered here the way the app answers it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, Context};
use postio_gtk::shell::ReaderOccupant;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::ThreadId;
use postio_model::{ContactId, ContactListRow, ContactSource, ContactState, ContactView};

fn people() -> Vec<ContactListRow> {
    ["Ada Lovelace", "Grace Hopper", "Katherine Johnson"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| ContactListRow {
            id: ContactId::new(i as i64 + 1),
            name: name.to_owned(),
            preferred: Some(format!("{}@example.com", name.split(' ').next().unwrap())),
            address_count: 1,
            last_seen_at: None,
            source: if i == 1 {
                ContactSource::User
            } else {
                ContactSource::Mail
            },
            state: ContactState::Live,
            sort_key: name.to_lowercase(),
        })
        .collect()
}

pub fn contacts_takes_the_pane_and_esc_gives_it_back() {
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

    // A conversation has the pane and the list has the keyboard: the state
    // Contacts has to hand back.
    let thread = ThreadId::new(7);
    let row = |id: i64| crate::gtk_reader_pane_owner::conversation_row(id, thread);
    window.show_conversation(vec![row(1), row(2)]);
    window.set_context(Context::List);
    pump();
    let conversation = window.conversation().widget();
    assert!(conversation.is_visible());

    // Answer the pane the way the app does.
    let pane = window.contacts();
    let queries: Rc<RefCell<Vec<ContactView>>> = Rc::default();
    let cursors: Rc<RefCell<Vec<Option<ContactId>>>> = Rc::default();
    pane.connect_query({
        let pane = pane.downgrade();
        let queries = queries.clone();
        move |view, _filter| {
            queries.borrow_mut().push(view);
            if let Some(pane) = pane.upgrade() {
                pane.reset(3);
            }
        }
    });
    pane.connect_page({
        let pane = pane.downgrade();
        move |_view, generation, page| {
            if let Some(pane) = pane.upgrade() {
                pane.deliver(generation, page, people(), 3);
            }
        }
    });
    pane.connect_cursor({
        let cursors = cursors.clone();
        move |person| cursors.borrow_mut().push(person)
    });

    window.act(Command::OpenContacts);
    pump();

    assert_eq!(window.shell().reader_occupant(), ReaderOccupant::Contacts);
    assert!(pane.is_visible(), "the screen is on screen");
    assert!(
        !conversation.is_visible(),
        "and it took the pane from the conversation rather than sitting under it"
    );
    assert_eq!(window.context(), Context::Contacts);
    assert_eq!(
        *queries.borrow(),
        [ContactView::Written],
        "the default view"
    );

    // The rows the list draws are the people the source answered with.
    let names: Vec<String> = (0..pane.model().n_items())
        .filter_map(|position| pane.model().row(position).map(|row| row.name))
        .collect();
    assert_eq!(names, ["Ada Lovelace", "Grace Hopper", "Katherine Johnson"]);
    assert_eq!(
        pane.cursor_person().map(|row| row.name).as_deref(),
        Some("Ada Lovelace"),
        "the cursor lands on the first person"
    );
    assert_eq!(
        cursors.borrow().last().copied().flatten(),
        Some(ContactId::new(1)),
        "and the app is asked for her detail"
    );

    // `v e` toggles to everyone from mail, and asks again.
    pane.dispatch(CommandId::ContactsToggleEveryone);
    pump();
    assert_eq!(
        *queries.borrow(),
        [ContactView::Written, ContactView::Everyone]
    );

    // `Esc`: the pane goes back to the conversation, the keyboard to the list.
    window.act(Command::Back);
    pump();
    assert!(!pane.is_open());
    assert!(!pane.is_visible());
    assert_eq!(
        window.shell().reader_occupant(),
        ReaderOccupant::Conversation
    );
    assert!(conversation.is_visible(), "computed, not replayed");
    assert_eq!(window.context(), Context::List);
    assert!(
        !window
            .shell()
            .has_css_class(postio_gtk::contacts::CONTACTS_OPEN_CLASS)
    );

    window.destroy();
}

pub fn a_draft_open_underneath_waits_for_contacts_to_close() {
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
    let composer = window.composer();
    composer.open(postio_model::Draft::new(postio_model::ids::AccountId::new(
        1,
    )));
    pump();
    assert!(composer.is_visible());

    window.act(Command::OpenContacts);
    pump();
    assert!(window.contacts().is_visible());
    assert!(
        !composer.is_visible(),
        "Contacts outranks the composer while open"
    );

    window.act(Command::Back);
    pump();
    assert!(composer.is_visible(), "and the draft is where it was left");
    assert!(composer.is_open());

    window.destroy();
}
