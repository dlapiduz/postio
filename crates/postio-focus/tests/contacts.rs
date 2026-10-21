//! The Contacts view, from the controller's side (specs/011-focus-contacts
//! S7, handoff screens 01 to 03): `g c` opens it in the list's place, the
//! counts, the pages and the card are read and stamped, the cursor and the
//! tabs and the filter are the controller's, and Escape unwinds them in
//! order.

use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId};
use postio_focus::{
    ContactsView, Effect, FocusController, Input, Intent, NoRows, Policy, Reply, Request,
    SurfaceKind, Ticket,
};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactArrival, ContactCardRead, ContactId, ContactKind,
    ContactListRow, ContactSettings, ContactSource, ContactState, ContactStats, ContactTab,
    ContactsOverview, ContactsPageRead, EmailAddress, ReviewSummary,
};
use postio_ui::keymap::KeyContext;

fn mac() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    let _ = focus.handle(Input::Keymap(postio_core::Keymap::resolve_on(
        &postio_config::KeyBindings::default(),
        Platform::Apple,
    )));
    focus
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

fn frame(effects: &[Effect]) -> Option<ContactsView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::Contacts(view) => Some(*view),
            _ => None,
        })
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

#[test]
fn g_c_opens_the_view_and_reads_the_counts() {
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::OpenContacts);
    assert_eq!(focus.key_context(), KeyContext::Contacts);
    assert_eq!(focus.top_surface(), Some(SurfaceKind::Contacts));
    let skeleton = frame(&effects).expect("a frame at once");
    assert!(!skeleton.loaded);
    assert!(matches!(
        &asks(&effects)[..],
        [(_, Request::ContactsOverview)]
    ));
}

#[test]
fn the_overview_fills_the_strip_and_the_first_page_gives_rows_in_letters() {
    let mut focus = mac();
    let effects = open(&mut focus);
    let view = frame(&effects).expect("the frame");
    assert!(view.loaded);
    assert_eq!(view.total, 3);
    assert_eq!(view.strip.title, "Contacts");
    assert_eq!(view.strip.tabs.len(), 5);
    assert!(
        view.strip.review.is_none(),
        "nothing to review hides the line"
    );
    assert_eq!(view.footer.status, "3 contacts · 1 saved");
    let first = focus.contacts_row(0).expect("row 0");
    assert_eq!(
        (first.name.as_str(), first.letter),
        ("Ada Moreno", Some('A'))
    );
    let second = focus.contacts_row(1).unwrap();
    assert_eq!(second.letter, None, "Adam is under the same A");
    let third = focus.contacts_row(2).unwrap();
    assert_eq!(third.letter, Some('B'));
    assert_eq!(focus.contacts_cursor(), Some(0));
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactsRows { first: 0, count: 3 })),
        "the rows that landed are told"
    );
}

#[test]
fn the_cursor_walks_the_list_and_the_card_follows_it() {
    let mut focus = mac();
    let effects = open(&mut focus);
    let [(ticket, Request::ContactCard { person, stamp })] = asks(&effects)[..] else {
        panic!("the card of the first row is read: {effects:?}");
    };
    assert_eq!(person, ContactId::new(1));
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactCard {
                stamp,
                answer: Ok(Some(Box::new(card(1, "Ada Moreno")))),
            },
        ),
        &NoRows,
    );
    assert!(shown(&effects).iter().any(
        |intent| matches!(intent, Intent::ContactsCard(Some(card)) if card.name == "Ada Moreno")
    ));

    let effects = run(&mut focus, CommandId::NextMessage);
    assert!(shown(&effects).contains(&Intent::ContactsCursor(Some(1))));
    let [(_, Request::ContactCard { person, .. })] = asks(&effects)[..] else {
        panic!("{effects:?}");
    };
    assert_eq!(person, ContactId::new(2));
    let _ = run(&mut focus, CommandId::NextMessage);
    assert_eq!(focus.contacts_cursor(), Some(2));
    let effects = run(&mut focus, CommandId::NextMessage);
    assert!(effects.is_empty(), "the end of the list: {effects:?}");
    let _ = run(&mut focus, CommandId::FirstMessage);
    assert_eq!(focus.contacts_cursor(), Some(0));
    let _ = run(&mut focus, CommandId::LastMessage);
    assert_eq!(focus.contacts_cursor(), Some(2));
    let _ = focus.handle_on(Input::ContactsPoint(1), &NoRows);
    assert_eq!(focus.contacts_cursor(), Some(1));
}

#[test]
fn a_card_that_answers_after_the_cursor_moved_on_is_dropped() {
    let mut focus = mac();
    let effects = open(&mut focus);
    let [(ticket, Request::ContactCard { stamp, .. })] = asks(&effects)[..] else {
        panic!("card");
    };
    let _ = run(&mut focus, CommandId::NextMessage);
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactCard {
                stamp,
                answer: Ok(Some(Box::new(card(1, "Ada Moreno")))),
            },
        ),
        &NoRows,
    );
    assert!(
        !shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::ContactsCard(Some(_)))),
        "Ada's card is not Adam's: {effects:?}"
    );
}

#[test]
fn a_tab_starts_the_list_over_and_a_stale_page_is_dropped() {
    let mut focus = mac();
    let _ = open(&mut focus);
    let wanted_before = {
        let mut other = mac();
        let _ = run(&mut other, CommandId::OpenContacts);
        other
    };
    drop(wanted_before);
    let effects = run(&mut focus, CommandId::ContactsTabNext);
    let view = frame(&effects).expect("the frame is drawn again");
    assert!(
        view.strip
            .tabs
            .iter()
            .any(|tab| tab.on && tab.title == "People")
    );
    assert_eq!(view.total, 3);
    assert!(focus.contacts_row(0).is_none(), "the old rows are gone");
    let wanted = focus.contacts_wanted(0);
    let [
        (
            ticket,
            Request::ContactsPage {
                tab, stamp, page, ..
            },
        ),
    ] = asks(&wanted)[..]
    else {
        panic!("{wanted:?}");
    };
    assert_eq!(tab, ContactTab::People);
    let stale = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactsPage {
                stamp: stamp - 1,
                page,
                answer: Ok(ContactsPageRead {
                    rows: people(),
                    before: None,
                }),
            },
        ),
        &NoRows,
    );
    assert!(stale.is_empty());
    assert!(focus.contacts_row(0).is_none());
    let _ = run(&mut focus, CommandId::ContactsTabPrevious);
    assert!(
        frame(&run(&mut focus, CommandId::ContactsTabPrevious)).is_none(),
        "All is the first tab: nothing moves"
    );
    let effects = focus.handle_on(Input::ContactsTab(ContactTab::Automated), &NoRows);
    assert!(
        frame(&effects)
            .unwrap()
            .strip
            .tabs
            .iter()
            .any(|t| t.on && t.title == "Automated")
    );
}

#[test]
fn the_filter_lists_what_it_finds_and_escape_unwinds_filter_then_view() {
    let mut focus = mac();
    let _ = open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactsFilter);
    assert!(shown(&effects).contains(&Intent::ContactsFocusFilter));
    let effects = focus.handle_on(Input::ContactsFilterTyped("ben".to_owned()), &NoRows);
    let found = asks(&effects);
    let [
        (
            ticket,
            Request::ContactsFiltered {
                text, stamp, tab, ..
            },
        ),
    ] = &found[..]
    else {
        panic!("{effects:?}");
    };
    let (ticket, stamp) = (*ticket, *stamp);
    assert_eq!((text.as_str(), *tab), ("ben", ContactTab::All));
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactsFiltered {
                stamp,
                answer: Ok(vec![row(3, "Ben Adeyemi", ContactKind::Person)]),
            },
        ),
        &NoRows,
    );
    assert_eq!(frame(&effects).unwrap().total, 1);
    assert_eq!(focus.contacts_row(0).unwrap().name, "Ben Adeyemi");
    assert_eq!(focus.contacts_cursor(), Some(0));

    // Escape clears the filter first, and the full list is back.
    let effects = run(&mut focus, CommandId::Back);
    let view = frame(&effects).expect("redrawn");
    assert_eq!((view.filter.as_str(), view.total), ("", 3));
    assert_eq!(focus.key_context(), KeyContext::Contacts);
    // Then it leaves.
    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Contacts)));
    assert!(shown(&effects).contains(&Intent::KeyboardHome));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn return_types_the_search_c_writes_and_i_opens_the_popover() {
    let mut focus = mac();
    let _ = open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactShowMail);
    assert!(
        shown(&effects).iter().any(|intent| matches!(
            intent,
            Intent::OpenBar { text, .. } if text == "with:ada.moreno@example.org"
        )),
        "{:?}",
        shown(&effects)
    );
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Contacts)));

    let mut focus = mac();
    let _ = open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactCompose);
    assert!(shown(&effects).contains(&Intent::WriteTo("ada.moreno@example.org".to_owned())));

    let mut focus = mac();
    let _ = open(&mut focus);
    let effects = run(&mut focus, CommandId::ContactPopover);
    assert!(matches!(
        &asks(&effects)[..],
        [(_, Request::ContactGlance { address, .. })] if address == "ada.moreno@example.org"
    ));
    assert_eq!(focus.key_context(), KeyContext::ContactPopover);
    let effects = run(&mut focus, CommandId::Back);
    assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::ContactPopover)));
    assert_eq!(
        focus.key_context(),
        KeyContext::Contacts,
        "the popover goes and the view is where it was"
    );
}

#[test]
fn the_catch_up_shows_in_the_footer_and_the_review_line_when_something_waits() {
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::OpenContacts);
    let [(ticket, _)] = asks(&effects)[..] else {
        panic!("asked");
    };
    let mut counted = overview(Some((1200, 1533)));
    counted.review = ReviewSummary {
        duplicates: 3,
        bounced: 0,
        frequent_unsaved: 5,
    };
    let effects = focus.handle_on(
        Input::Reply(ticket, Reply::ContactsOverview(Ok(counted))),
        &NoRows,
    );
    let view = frame(&effects).unwrap();
    assert!(
        view.footer
            .status
            .starts_with("Building contacts, 1,200 of 1,533")
    );
    let line = view.strip.review.expect("something is worth a look");
    assert_eq!(
        line.text,
        "3 contacts may be the same person · 5 people you write to often aren't saved"
    );
}

#[test]
fn a_change_to_the_people_reads_the_counts_again_and_gtk_has_no_such_view() {
    let mut focus = mac();
    let _ = open(&mut focus);
    let effects = focus.handle_on(
        Input::Event(postio_core::Event::ContactsChanged {
            names_changed: true,
        }),
        &NoRows,
    );
    assert!(
        asks(&effects)
            .iter()
            .any(|(_, request)| matches!(request, Request::ContactsOverview))
    );

    let mut linux = FocusController::new(Policy::for_platform(Platform::Freedesktop));
    let effects = run(&mut linux, CommandId::OpenContacts);
    assert!(
        effects.is_empty(),
        "Linux adopts Contacts later: {effects:?}"
    );
    assert_eq!(linux.key_context(), KeyContext::List);
}

#[test]
fn other_keys_do_nothing_under_the_view() {
    let mut focus = mac();
    let _ = open(&mut focus);
    for command in [CommandId::Archive, CommandId::Snooze] {
        let effects = run(&mut focus, command);
        assert!(effects.is_empty(), "{command:?}: {effects:?}");
    }
    let _ = Command::Undo;
}
