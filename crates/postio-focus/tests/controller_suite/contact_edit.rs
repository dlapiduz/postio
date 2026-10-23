//! The Edit and New contact form, from the controller's side
//! (specs/011-focus-contacts S8, handoff screen 06): the Mac's keys open it,
//! Escape arms before it discards, Save is one command or says why not, and
//! the card's switches are the same kind of command.

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId};
use postio_focus::{
    EditChange, Effect, FocusController, Input, Intent, NoRows, Policy, Reply, Request, SurfaceKind,
};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactArrival, ContactCardRead, ContactId, ContactKind,
    ContactListRow, ContactSettings, ContactSource, ContactState, ContactStats, ContactTab,
    ContactsOverview, ContactsPageRead, EmailAddress, ReviewSummary,
};

fn mac() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    let _ = focus.handle(Input::Keymap(postio_core::Keymap::resolve_on(
        &postio_config::KeyBindings::default(),
        Platform::Apple,
    )));
    focus
}

fn run(focus: &mut FocusController, command: CommandId) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), &NoRows)
}

fn row(n: i64, name: &str, kind: ContactKind) -> ContactListRow {
    ContactListRow {
        id: ContactId::new(n),
        name: name.to_owned(),
        preferred: Some(format!(
            "{}@example.org",
            name.to_lowercase().replace(' ', ".")
        )),
        address_count: 1,
        last_seen_at: Some(Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap()),
        source: ContactSource::Mail,
        state: ContactState::Live,
        sort_key: name.to_lowercase(),
        kind,
        messages: 10 * n as u32,
        saved: false,
        arrival: None,
        notify: false,
        html_original: false,
        digest_cadence: None,
    }
}

fn people() -> Vec<ContactListRow> {
    vec![
        row(1, "Ada Moreno", ContactKind::Person),
        row(2, "Adam Okoro", ContactKind::Person),
        row(3, "Ben Adeyemi", ContactKind::Person),
    ]
}

fn overview(building: Option<(u32, u32)>) -> ContactsOverview {
    ContactsOverview {
        total: 3,
        saved: 1,
        tabs: vec![
            (ContactTab::All, 3),
            (ContactTab::People, 3),
            (ContactTab::Companies, 0),
            (ContactTab::Lists, 0),
            (ContactTab::Automated, 0),
        ],
        review: ReviewSummary::default(),
        building,
    }
}

fn card(id: i64, name: &str) -> ContactCardRead {
    let mut person = Contact::new(ContactAddress::new(
        AddressId::new(id),
        EmailAddress::new(
            Some(name),
            format!("{}@example.org", name.to_lowercase().replace(' ', ".")),
        ),
    ));
    person.id = ContactId::new(id);
    ContactCardRead {
        person,
        groups: Vec::new(),
        stats: ContactStats::default(),
        open: Vec::new(),
        settings: ContactSettings::default(),
        default_arrival: ContactArrival::Inbox,
        files: Vec::new(),
        issues: Vec::new(),
        address_notes: Vec::new(),
    }
}

/// `g c`, the overview answered, page 0 read: the view as a person sees it.
fn open(focus: &mut FocusController) -> Vec<Effect> {
    let effects = run(focus, CommandId::OpenContacts);
    let [(ticket, Request::ContactsOverview)] = asks(&effects)[..] else {
        panic!("the counts are read: {effects:?}");
    };
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactsOverview(Ok(overview(None)))),
        &NoRows,
    );
    let wanted = focus.contacts_wanted(0);
    let [(ticket, Request::ContactsPage { stamp, page, .. })] = asks(&wanted)[..] else {
        panic!("page 0 is wanted: {wanted:?}");
    };
    let mut effects = effects;
    effects.extend(focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactsPage {
                stamp,
                page,
                answer: Ok(ContactsPageRead {
                    rows: people(),
                    before: None,
                }),
            },
        ),
        &NoRows,
    ));
    effects
}

/// The view open with the first person's card read: where ⌘E has someone.
fn open_with_card(focus: &mut FocusController) {
    let effects = open(focus);
    let [(ticket, Request::ContactCard { stamp, .. })] = asks(&effects)[..] else {
        panic!("the first card is read: {effects:?}");
    };
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactCard {
                stamp,
                answer: Ok(Some(Box::new(card(1, "Ada Moreno")))),
            },
        ),
        &NoRows,
    );
}

fn form(effects: &[Effect]) -> Option<postio_ui::contact_edit::EditView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ContactEdit(view) => view.map(|view| *view),
            _ => None,
        })
}

fn change(focus: &mut FocusController, change: EditChange) -> Vec<Effect> {
    focus.handle_on(Input::ContactEditChange(change), &NoRows)
}

fn posted(effects: &[Effect]) -> Vec<Command> {
    asks(effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            Request::Post(command) => Some(command),
            _ => None,
        })
        .collect()
}

#[test]
fn cmd_e_opens_the_form_on_the_person_under_the_cursor() {
    let mut focus = mac();
    open_with_card(&mut focus);
    let effects = run(&mut focus, CommandId::ContactEdit);
    let view = form(&effects).expect("the form is drawn");
    assert_eq!(view.title, "Editing Ada Moreno");
    assert_eq!(view.given, "Ada");
    assert_eq!(view.family, "Moreno");
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Contacts));
    assert!(focus.contact_edit().is_some());
}

#[test]
fn cmd_e_before_the_card_is_read_does_nothing() {
    let mut focus = mac();
    open(&mut focus);
    assert!(form(&run(&mut focus, CommandId::ContactEdit)).is_none());
    assert!(focus.contact_edit().is_none());
}

#[test]
fn cmd_shift_n_opens_an_empty_form_for_a_new_person() {
    let mut focus = mac();
    open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactNew);
    let view = form(&effects).expect("the form is drawn");
    assert_eq!(view.title, "New contact");
    assert_eq!(view.addresses.len(), 1, "one empty address row to type in");
    assert_eq!(view.given, "");
}

#[test]
fn escape_on_an_untouched_form_closes_it_at_once() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    let effects = run(&mut focus, CommandId::Back);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactEdit(None)))
    );
    assert!(focus.contact_edit().is_none());
    assert_eq!(
        focus.top_surface(),
        Some(SurfaceKind::Contacts),
        "the view stays"
    );
}

#[test]
fn escape_on_an_edited_form_arms_first_and_a_second_discards() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    change(&mut focus, EditChange::Given("Adele".into()));

    let effects = run(&mut focus, CommandId::Back);
    let armed = form(&effects).expect("redrawn, armed");
    assert!(armed.armed.is_some(), "it says Esc again discards");
    assert!(focus.contact_edit().is_some(), "still open");

    let effects = run(&mut focus, CommandId::Back);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactEdit(None)))
    );
    assert!(posted(&effects).is_empty(), "nothing was saved");
}

#[test]
fn typing_after_arming_disarms_it() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    change(&mut focus, EditChange::Given("Adele".into()));
    run(&mut focus, CommandId::Back);
    let effects = change(&mut focus, EditChange::Family("Moreno-Diaz".into()));
    assert!(form(&effects).expect("redrawn").armed.is_none());
    let effects = run(&mut focus, CommandId::Back);
    assert!(
        form(&effects).expect("armed again").armed.is_some(),
        "the first Esc after typing arms again"
    );
}

#[test]
fn changing_a_field_back_to_what_it_was_is_no_change() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    change(&mut focus, EditChange::Given("Adele".into()));
    change(&mut focus, EditChange::Given("Ada".into()));
    let effects = run(&mut focus, CommandId::Back);
    assert!(
        focus.contact_edit().is_none(),
        "closed without arming: {effects:?}"
    );
}

#[test]
fn cmd_s_saves_the_form_as_one_command_and_closes_it() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    change(&mut focus, EditChange::Title("Finance lead".into()));
    change(&mut focus, EditChange::AddGroup("Finance".into()));
    let effects = run(&mut focus, CommandId::ContactSave);
    let [
        Command::ContactEdit {
            action: postio_core::ContactEditAction::Save { person, form },
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one save is posted: {effects:?}");
    };
    assert_eq!(*person, Some(ContactId::new(1)));
    assert_eq!(form.title, "Finance lead");
    assert_eq!(form.groups, ["Finance"]);
    assert!(focus.contact_edit().is_none(), "the form closes on Save");
}

#[test]
fn cmd_s_on_a_form_that_cannot_be_saved_says_why_and_stays() {
    let mut focus = mac();
    open(&mut focus);
    run(&mut focus, CommandId::ContactNew);
    change(&mut focus, EditChange::Given("Grace".into()));
    let effects = run(&mut focus, CommandId::ContactSave);
    assert!(posted(&effects).is_empty());
    let view = form(&effects).expect("redrawn with the reason");
    assert_eq!(view.problem.as_deref(), Some("Add at least one address."));

    change(
        &mut focus,
        EditChange::Address {
            index: 0,
            text: "grace@example.org".into(),
        },
    );
    let effects = run(&mut focus, CommandId::ContactSave);
    let [
        Command::ContactEdit {
            action: postio_core::ContactEditAction::Save { person: None, .. },
        },
    ] = &posted(&effects)[..]
    else {
        panic!("a new person is saved: {effects:?}");
    };
}

#[test]
fn the_address_buttons_add_remove_and_choose_the_primary() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    let effects = change(&mut focus, EditChange::AddAddress);
    assert_eq!(form(&effects).unwrap().addresses.len(), 2);
    change(
        &mut focus,
        EditChange::Address {
            index: 1,
            text: "ada@example.net".into(),
        },
    );
    let effects = change(&mut focus, EditChange::Primary(1));
    let view = form(&effects).unwrap();
    assert!(view.addresses[1].primary && !view.addresses[0].primary);
    let effects = change(&mut focus, EditChange::RemoveAddress(0));
    let view = form(&effects).unwrap();
    assert_eq!(view.addresses.len(), 1);
    assert!(view.addresses[0].primary, "the primary follows its address");
    assert_eq!(view.addresses[0].text, "ada@example.net");
}

#[test]
fn other_keys_do_nothing_under_the_form() {
    let mut focus = mac();
    open_with_card(&mut focus);
    run(&mut focus, CommandId::ContactEdit);
    let before = focus.contacts_cursor();
    run(&mut focus, CommandId::NextMessage);
    assert_eq!(
        focus.contacts_cursor(),
        before,
        "j moves nothing behind the form"
    );
}

#[test]
fn a_card_switch_is_one_undoable_command_and_draws_at_once() {
    let mut focus = mac();
    open_with_card(&mut focus);
    let card = focus.contacts_card().expect("the card");
    let at = card
        .switches
        .iter()
        .position(|line| line.switch == Some(postio_model::CardSwitch::Notify))
        .expect("a notify switch");
    assert!(!card.switches[at].on);
    let effects = focus.handle_on(Input::ContactCardSwitch(at as u32), &NoRows);
    let [
        Command::ContactEdit {
            action:
                postio_core::ContactEditAction::Switch {
                    address,
                    switch,
                    on: true,
                },
        },
    ] = &posted(&effects)[..]
    else {
        panic!("a switch is posted: {effects:?}");
    };
    assert_eq!(address, "ada.moreno@example.org");
    assert_eq!(*switch, postio_model::CardSwitch::Notify);
    assert!(
        focus.contacts_card().unwrap().switches[at].on,
        "drawn at once"
    );
}

#[test]
fn the_group_picker_offers_what_exists_filtered_and_picks_by_index() {
    let mut focus = mac();
    open_with_card(&mut focus);
    let effects = run(&mut focus, CommandId::ContactEdit);
    let ticket = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| matches!(request, Request::ContactGroups).then_some(ticket))
        .expect("the groups are read when the form opens");
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactGroups(Ok(vec!["Atlas core".into(), "Finance".into()])),
        ),
        &NoRows,
    );
    let effects = change(&mut focus, EditChange::GroupQuery(Some("fin".into())));
    let picker = form(&effects).unwrap().group_picker.expect("open");
    let labels: Vec<&str> = picker.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Finance", "New group fin"]);

    let effects = change(&mut focus, EditChange::GroupPick(0));
    let view = form(&effects).unwrap();
    assert_eq!(view.groups, ["Finance"]);
    assert!(view.group_picker.is_none(), "picking puts the picker away");

    let effects = change(&mut focus, EditChange::GroupQuery(Some("Book club".into())));
    let options = form(&effects).unwrap().group_picker.unwrap().options;
    assert_eq!(options.last().unwrap().label, "New group Book club");
    let effects = change(&mut focus, EditChange::GroupPick(options.len() as u32 - 1));
    assert_eq!(form(&effects).unwrap().groups, ["Finance", "Book club"]);
}

#[test]
fn seen_in_mail_is_read_when_the_form_opens_and_add_and_ignore_are_commands() {
    use postio_model::{DetailKind, SeenItem};
    let mut focus = mac();
    open_with_card(&mut focus);
    let effects = run(&mut focus, CommandId::ContactEdit);
    let seen_ticket = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ContactSeen(person) if person == ContactId::new(1) => Some(ticket),
            _ => None,
        })
        .expect("the mail's details are read for the person");
    let effects = focus.handle_on(
        Input::Reply(
            seen_ticket,
            Reply::ContactSeen(vec![
                SeenItem {
                    kind: DetailKind::Phone,
                    value: "+1 555 0119".into(),
                    kept: false,
                },
                SeenItem {
                    kind: DetailKind::Url,
                    value: "northwind.example".into(),
                    kept: false,
                },
            ]),
        ),
        &NoRows,
    );
    let rows = form(&effects).unwrap().seen;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].action, "Add phone");

    let effects = change(
        &mut focus,
        EditChange::Seen {
            index: 0,
            secondary: false,
        },
    );
    let [
        Command::ContactEdit {
            action:
                postio_core::ContactEditAction::Detail {
                    kind,
                    value,
                    keep: Some(true),
                    ..
                },
        },
    ] = &posted(&effects)[..]
    else {
        panic!("a kept phone is a command: {effects:?}");
    };
    assert_eq!((*kind, value.as_str()), (DetailKind::Phone, "+1 555 0119"));
    let rows = form(&effects).unwrap().seen;
    assert_eq!(
        rows.iter().filter(|r| r.action == "Remove").count(),
        1,
        "kept, so it can be removed"
    );

    let effects = change(
        &mut focus,
        EditChange::Seen {
            index: 0,
            secondary: true,
        },
    );
    assert!(matches!(
        &posted(&effects)[..],
        [Command::ContactEdit {
            action: postio_core::ContactEditAction::Detail {
                keep: Some(false),
                ..
            }
        }]
    ));
    // The form is not made dirty by keeping a detail: Esc closes at once.
    let effects = run(&mut focus, CommandId::Back);
    assert!(focus.contact_edit().is_none(), "{effects:?}");
}
