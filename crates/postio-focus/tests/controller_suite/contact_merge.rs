//! The review queue and the merge sheet, from the controller's side
//! (specs/011-focus-contacts S10, handoff screen 07): `g w` reads the queue
//! and opens the sheet, `x` marks and `m` merges the marked, Return merges,
//! `n` says they are two people, Escape is Later, and the arrows choose whose
//! name is kept.

use crate::search_support::{asks, shown};
use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId};
use postio_focus::{
    Effect, FocusController, Input, Intent, NoRows, Policy, Reply, Request, SurfaceKind,
};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactArrival, ContactCardRead, ContactId, ContactKind,
    ContactListRow, ContactSettings, ContactSource, ContactState, ContactStats, ContactTab,
    ContactsOverview, ContactsPageRead, EmailAddress, MergeGroup, ReviewSummary,
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

fn merge_person(id: i64, name: &str, address: &str, seen: u32, saved: bool) -> Contact {
    let mut person = Contact::new(ContactAddress::new(
        AddressId::new(id),
        EmailAddress::new(Some(name), address),
    ));
    person.id = ContactId::new(id);
    person.times_seen = seen;
    if saved {
        person.saved_at = Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
    }
    person
}

fn queue() -> Vec<MergeGroup> {
    vec![
        MergeGroup {
            people: vec![
                merge_person(1, "Ada Moreno", "ada@example.org", 412, true),
                merge_person(2, "A. Moreno", "a.moreno@example.net", 14, false),
            ],
            evidence: vec!["Both are called \u{201c}Ada Moreno\u{201d}".to_owned()],
        },
        MergeGroup {
            people: vec![
                merge_person(3, "Ben Adeyemi", "ben@example.org", 80, true),
                merge_person(4, "Ben A.", "ben@example.net", 3, false),
            ],
            evidence: vec!["The same signature on both".to_owned()],
        },
    ]
}

fn sheet(effects: &[Effect]) -> Option<postio_ui::contact_merge::MergeSheetView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ContactMerge(view) => view.map(|view| *view),
            _ => None,
        })
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

/// `g w`, answered with the queue.
fn review(focus: &mut FocusController) -> Vec<Effect> {
    open(focus);
    let effects = run(focus, CommandId::ContactsSuggestions);
    let ticket = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| {
            matches!(request, Request::ContactReview { .. }).then_some(ticket)
        })
        .expect("the queue is read");
    focus.handle_on(Input::Reply(ticket, Reply::ContactReview(queue())), &NoRows)
}

#[test]
fn g_w_reads_the_queue_and_opens_the_sheet_on_its_first_group() {
    let mut focus = mac();
    let effects = review(&mut focus);
    let view = sheet(&effects).expect("the sheet is drawn");
    assert_eq!(view.title, "These 2 look like the same person");
    assert_eq!(view.position.as_deref(), Some("1 of 2"));
    assert_eq!(view.cards.iter().filter(|c| c.kept).count(), 1);
    assert_eq!(view.cards[0].name, "Ada Moreno");
}

#[test]
fn an_empty_queue_opens_nothing() {
    let mut focus = mac();
    open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactsSuggestions);
    let ticket = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| {
            matches!(request, Request::ContactReview { .. }).then_some(ticket)
        })
        .unwrap();
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactReview(Vec::new())),
        &NoRows,
    );
    assert!(sheet(&effects).is_none());
}

#[test]
fn the_arrows_choose_whose_name_is_kept_and_a_click_does_too() {
    let mut focus = mac();
    review(&mut focus);
    let effects = run(&mut focus, CommandId::NextMessage);
    let view = sheet(&effects).unwrap();
    assert!(view.cards[1].kept && view.result.name == "A. Moreno");
    let effects = run(&mut focus, CommandId::PrevMessage);
    assert!(sheet(&effects).unwrap().cards[0].kept);
    let effects = focus.handle_on(Input::ContactMergeKeep(1), &NoRows);
    assert!(sheet(&effects).unwrap().cards[1].kept);
}

#[test]
fn return_merges_into_the_kept_record_and_shows_the_next_group() {
    let mut focus = mac();
    review(&mut focus);
    let effects = run(&mut focus, CommandId::ContactShowMail);
    let [
        Command::ContactJoin {
            action:
                postio_core::ContactJoinAction::Join {
                    into, others, name, ..
                },
        },
    ] = &posted(&effects)[..]
    else {
        panic!("one merge is posted: {effects:?}");
    };
    assert_eq!(*into, ContactId::new(1));
    assert_eq!(others, &[ContactId::new(2)]);
    assert_eq!(name, "Ada Moreno");
    let next = sheet(&effects).expect("the next group is drawn");
    assert_eq!(next.cards[0].name, "Ben Adeyemi");
    assert_eq!(next.position.as_deref(), Some("2 of 2"));

    let effects = run(&mut focus, CommandId::ContactShowMail);
    assert_eq!(posted(&effects).len(), 1);
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactMerge(None))),
        "the last group closes the sheet"
    );
}

#[test]
fn n_says_they_are_two_people_and_moves_on() {
    let mut focus = mac();
    review(&mut focus);
    let effects = run(&mut focus, CommandId::SuggestionDismiss);
    let [Command::SuggestionDismiss { pair: Some((a, b)) }] = &posted(&effects)[..] else {
        panic!("one dismissal is posted: {effects:?}");
    };
    assert_eq!((a.get(), b.get()), (1, 2));
    assert_eq!(sheet(&effects).unwrap().cards[0].name, "Ben Adeyemi");
}

#[test]
fn escape_is_later_and_sends_nothing() {
    let mut focus = mac();
    review(&mut focus);
    let effects = run(&mut focus, CommandId::Back);
    assert!(posted(&effects).is_empty());
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactMerge(None)))
    );
    assert_eq!(
        focus.top_surface(),
        Some(SurfaceKind::Contacts),
        "the view stays"
    );
}

#[test]
fn x_marks_people_and_m_merges_the_marked_through_the_same_sheet() {
    let mut focus = mac();
    open(&mut focus);
    assert!(
        run(&mut focus, CommandId::ContactJoin).is_empty()
            || asks(&run(&mut focus, CommandId::ContactJoin)).is_empty()
    );
    run(&mut focus, CommandId::ToggleSelection);
    assert!(
        focus.contacts_row(0).unwrap().marked,
        "the first row is marked"
    );
    run(&mut focus, CommandId::NextMessage);
    run(&mut focus, CommandId::ToggleSelection);
    let effects = run(&mut focus, CommandId::ContactJoin);
    let (ticket, ids) = asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ContactMergeOf(ids) => Some((ticket, ids)),
            _ => None,
        })
        .expect("the marked are asked about");
    assert_eq!(ids, [ContactId::new(1), ContactId::new(2)]);
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactMergeOf(Some(Box::new(queue().remove(0)))),
        ),
        &NoRows,
    );
    let view = sheet(&effects).expect("the sheet");
    assert_eq!(view.position, None, "chosen by hand, so not in a queue");
    let effects = run(&mut focus, CommandId::ContactShowMail);
    assert_eq!(posted(&effects).len(), 1);
    assert!(
        !focus.contacts_row(0).unwrap().marked,
        "merged, so unmarked"
    );
}

#[test]
fn m_with_fewer_than_two_marked_does_nothing() {
    let mut focus = mac();
    open(&mut focus);
    run(&mut focus, CommandId::ToggleSelection);
    let effects = run(&mut focus, CommandId::ContactJoin);
    assert!(asks(&effects).is_empty(), "{effects:?}");
}

#[test]
fn other_keys_do_nothing_under_the_sheet() {
    let mut focus = mac();
    review(&mut focus);
    let before = focus.contacts_cursor();
    run(&mut focus, CommandId::ContactCompose);
    run(&mut focus, CommandId::ContactEdit);
    assert_eq!(focus.contacts_cursor(), before);
    assert!(focus.contact_edit().is_none());
}

#[test]
fn g_w_from_the_inbox_opens_contacts_and_reads_the_queue() {
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::ContactsSuggestions);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Contacts));
    assert!(
        asks(&effects)
            .iter()
            .any(|(_, request)| matches!(request, Request::ContactReview { .. }))
    );
}

fn card_with(addresses: &[(i64, &str)]) -> ContactCardRead {
    let mut read = card(1, "Ada Moreno");
    read.person.addresses = addresses
        .iter()
        .map(|(id, address)| {
            ContactAddress::new(
                AddressId::new(*id),
                EmailAddress::new(None::<String>, *address),
            )
        })
        .collect();
    read
}

fn focus_on_card(read: ContactCardRead) -> FocusController {
    let mut focus = mac();
    let effects = open(&mut focus);
    let [(ticket, Request::ContactCard { stamp, .. })] = asks(&effects)[..] else {
        panic!("the first card is read");
    };
    focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactCard {
                stamp,
                answer: Ok(Some(Box::new(read))),
            },
        ),
        &NoRows,
    );
    focus
}

#[test]
fn split_takes_an_address_off_a_merged_person_and_a_lone_address_stays() {
    let mut lone = focus_on_card(card_with(&[(1, "ada@example.org")]));
    let effects = lone.handle_on(Input::ContactSplit("ada@example.org".into()), &NoRows);
    assert!(posted(&effects).is_empty(), "one address is not split off");

    let mut merged = focus_on_card(card_with(&[
        (1, "ada@example.org"),
        (9, "a.moreno@example.net"),
    ]));
    let effects = merged.handle_on(Input::ContactSplit("A.Moreno@example.net".into()), &NoRows);
    let [Command::ContactDetachAddress { address: Some(id) }] = &posted(&effects)[..] else {
        panic!("a split is one command: {effects:?}");
    };
    assert_eq!(id.get(), 9);
}

/// An import lands with the view up: the counts are read again, and then
/// where the first new person is. Returns that question.
fn imported(
    focus: &mut FocusController,
    made: Vec<ContactId>,
    touched: Vec<ContactId>,
) -> (postio_focus::Ticket, ContactId) {
    let effects = focus.handle_on(Input::ContactsImported { made, touched }, &NoRows);
    let [(ticket, Request::ContactsOverview)] = asks(&effects)[..] else {
        panic!("the counts are read before the position: {effects:?}");
    };
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactsOverview(Ok(overview(None)))),
        &NoRows,
    );
    asks(&effects)
        .into_iter()
        .find_map(|(ticket, request)| match request {
            Request::ContactPosition { person, .. } => Some((ticket, person)),
            _ => None,
        })
        .expect("where the first new person is, is asked")
}

#[test]
fn after_an_import_the_cursor_goes_to_the_first_new_person_and_the_review_looks_at_what_it_found() {
    let mut focus = mac();
    open(&mut focus);
    let (ticket, person) = imported(
        &mut focus,
        vec![ContactId::new(3)],
        vec![ContactId::new(3), ContactId::new(1)],
    );
    assert_eq!(person, ContactId::new(3));
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactPosition {
                person,
                at: Some(2),
            },
        ),
        &NoRows,
    );
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactsCursor(Some(2)))),
        "the cursor is on the new person: {effects:?}"
    );
    assert_eq!(focus.contacts_cursor(), Some(2));

    // The review looks at what the import found, once.
    let effects = run(&mut focus, CommandId::ContactsSuggestions);
    let first = asks(&effects)
        .into_iter()
        .find_map(|(_, request)| match request {
            Request::ContactReview { first } => Some(first),
            _ => None,
        })
        .expect("the review is asked for");
    assert_eq!(first, [ContactId::new(3), ContactId::new(1)]);
    let again = run(&mut focus, CommandId::ContactsSuggestions);
    let first = asks(&again)
        .into_iter()
        .find_map(|(_, request)| match request {
            Request::ContactReview { first } => Some(first),
            _ => None,
        })
        .expect("asked again");
    assert!(
        first.is_empty(),
        "an import's candidates lead the review once"
    );
}

#[test]
fn an_import_with_contacts_closed_remembers_where_to_go_and_what_to_review() {
    let mut focus = mac();
    let effects = focus.handle_on(
        Input::ContactsImported {
            made: vec![ContactId::new(3)],
            touched: vec![ContactId::new(3)],
        },
        &NoRows,
    );
    assert!(asks(&effects).is_empty(), "nothing to move yet");
    let effects = run(&mut focus, CommandId::ContactsSuggestions);
    assert!(
        asks(&effects).iter().any(|(_, request)| matches!(
            request,
            Request::ContactReview { first } if first == &[ContactId::new(3)]
        )),
        "{effects:?}"
    );
}

/// Bringing people in from the Mac's Contacts says the people changed (the
/// engine's `ContactsChanged`) and then which of them are new. With the view
/// up they are on screen: the rows are read again and the cursor goes to the
/// first one, even when it is not on the tab being shown.
#[test]
fn people_brought_in_from_contacts_are_shown_with_the_view_up() {
    let mut focus = mac();
    open(&mut focus);
    let changed = focus.handle_on(
        Input::Event(postio_core::Event::ContactsChanged {
            names_changed: true,
        }),
        &NoRows,
    );
    assert!(
        shown(&changed)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactsRows { .. })),
        "the list reads its rows again: {changed:?}"
    );
    let (ticket, person) = imported(
        &mut focus,
        vec![ContactId::new(7), ContactId::new(8)],
        vec![ContactId::new(7), ContactId::new(8), ContactId::new(1)],
    );
    assert_eq!(person, ContactId::new(7));

    // On another tab the person is not in the list; the list is still current.
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactPosition { person, at: None }),
        &NoRows,
    );
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactsRows { .. })),
        "a person on another tab still leaves the rows current: {effects:?}"
    );
}

/// An import changes the people and says who is new; the engine's
/// `ContactsChanged` re-reads the counts. However those answers arrive, the
/// cursor ends on the first new person: a position answered before the new
/// counts must not be undone by the restart they cause (#2027, where the
/// GTK suite saw the cursor stay on the first row).
#[test]
fn the_cursor_reaches_the_imported_person_when_the_new_counts_come_last() {
    let mut focus = mac();
    open(&mut focus);
    let mut pending = asks(&focus.handle_on(
        Input::Event(postio_core::Event::ContactsChanged {
            names_changed: true,
        }),
        &NoRows,
    ));
    pending.extend(asks(&focus.handle_on(
        Input::ContactsImported {
            made: vec![ContactId::new(4)],
            touched: vec![ContactId::new(4)],
        },
        &NoRows,
    )));
    let mut grown = overview(None);
    grown.total = 4;
    for (tab, count) in &mut grown.tabs {
        if matches!(tab, ContactTab::All | ContactTab::People) {
            *count = 4;
        }
    }
    // Positions first: the order that loses the cursor.
    for _ in 0..8 {
        pending.sort_by_key(|(_, request)| !matches!(request, Request::ContactPosition { .. }));
        let Some(index) = pending.iter().position(|(_, request)| {
            matches!(
                request,
                Request::ContactPosition { .. } | Request::ContactsOverview
            )
        }) else {
            break;
        };
        let (ticket, request) = pending.remove(index);
        let reply = match request {
            Request::ContactPosition { person, .. } => Reply::ContactPosition {
                person,
                at: Some(3),
            },
            _ => Reply::ContactsOverview(Ok(grown.clone())),
        };
        pending.extend(asks(&focus.handle_on(Input::Reply(ticket, reply), &NoRows)));
    }
    assert_eq!(focus.contacts_total(), 4);
    assert_eq!(
        focus.contacts_cursor(),
        Some(3),
        "the cursor is on the imported person, not where the restart put it"
    );
}
