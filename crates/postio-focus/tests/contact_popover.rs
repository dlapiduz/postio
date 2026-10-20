//! The known-sender popover, from the controller's side
//! (specs/011-focus-contacts S6, handoff screen 04).
//!
//! `i` on a row, or a click on a sender's name, asks the host for the person
//! at a glance and draws what it answers; the arrow keys change where their
//! mail arrives, at once; Return is all their mail, `c` writes to them, and
//! Escape puts it away. What is drawn is `postio_ui::contact_popover`'s, so
//! these assert the keys and the asks, and a few words a person would read.

use chrono::{TimeZone, Utc};
use postio_config::paths::Platform;
use postio_core::{Command, CommandId};
use postio_focus::{
    Effect, FocusController, Input, Intent, Policy, Reply, Request, RowFacts, Rows, SurfaceKind,
    Ticket,
};
use postio_model::listing::{MessageSummary, ThreadSummary};
use postio_model::{
    AddressId, Contact, ContactAddress, ContactArrival, ContactGlance, ContactId, ContactStats,
    EmailAddress, MessageId, OpenItem, OpenSide, ThreadId,
};
use postio_ui::contact_popover::PopoverView;
use postio_ui::focus_list::FocusRow;
use postio_ui::keymap::KeyContext;

/// One conversation from Ada.
struct List;

impl Rows for List {
    fn len(&self) -> u32 {
        1
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.row(position).map(|row| RowFacts::of(&row))
    }
    fn position_of(&self, id: MessageId) -> Option<u32> {
        (id == MessageId::new(42)).then_some(0)
    }
    fn row(&self, position: u32) -> Option<FocusRow> {
        (position == 0).then(|| {
            let at = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();
            FocusRow::conversation(ThreadSummary {
                id: Some(ThreadId::new(7)),
                representative: MessageSummary {
                    id: MessageId::new(42),
                    thread: Some(ThreadId::new(7)),
                    from: Some(EmailAddress::new(Some("Ada Moreno"), "ada@example.com")),
                    subject: Some("Quarterly review".to_owned()),
                    preview: None,
                    received_at: at,
                    seen: false,
                    flagged: false,
                    answered: false,
                    send_state: None,
                    send_at: None,
                    has_attachments: false,
                    thread_count: 1,
                    to: Vec::new(),
                },
                subject: Some("Quarterly review".to_owned()),
                participants: Vec::new(),
                message_count: 1,
                unread_count: 1,
                flagged: false,
                has_attachments: false,
                last_at: at,
                marker: None,
                copies: Vec::new(),
            })
        })
    }
}

fn ready() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    let _ = focus.handle(Input::Keymap(postio_core::Keymap::resolve_on(
        &postio_config::KeyBindings::default(),
        Platform::Apple,
    )));
    let _ = focus.landed(&List, true);
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

fn drawn(effects: &[Effect]) -> Option<PopoverView> {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::ContactPopover(view) => Some(*view),
            _ => None,
        })
}

fn run(focus: &mut FocusController, command: CommandId) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), &List)
}

fn ada(arrival: Option<ContactArrival>) -> ContactGlance {
    let mut person = Contact::new(ContactAddress::new(
        AddressId::new(1),
        EmailAddress::new(Some("Ada Moreno"), "ada@example.com"),
    ));
    person.id = ContactId::new(7);
    person.title = Some("Finance lead".to_owned());
    person.organization = Some("Northwind Partners".to_owned());
    ContactGlance {
        person,
        stats: ContactStats {
            messages: 431,
            from_you: 88,
            opened: 400,
            last: Some(Utc::now()),
            months: Vec::new(),
            recent: Vec::new(),
        },
        open: vec![OpenItem {
            side: OpenSide::WaitingOnYou,
            text: "Approve the Q3 numbers by Friday".to_owned(),
            message: MessageId::new(42),
        }],
        arrival,
        default_arrival: ContactArrival::Inbox,
    }
}

/// `i`, answered with Ada: the popover as it is drawn.
fn open(focus: &mut FocusController, arrival: Option<ContactArrival>) -> PopoverView {
    let effects = run(focus, CommandId::ContactPopover);
    let [(ticket, Request::ContactGlance { stamp, .. })] = asks(&effects)[..] else {
        panic!("the host is asked: {effects:?}");
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactGlance {
                stamp,
                glance: Ok(Some(Box::new(ada(arrival)))),
            },
        ),
        &List,
    );
    drawn(&effects).expect("the popover is drawn")
}

fn on(view: &PopoverView) -> Vec<&str> {
    view.arrival
        .iter()
        .filter(|option| option.on)
        .map(|option| option.label.as_str())
        .collect()
}

#[test]
fn i_asks_about_the_rows_sender_and_draws_nothing_until_it_is_answered() {
    let mut focus = ready();
    let effects = run(&mut focus, CommandId::ContactPopover);
    assert!(
        matches!(
            &asks(&effects)[..],
            [(_, Request::ContactGlance { address, .. })] if address == "ada@example.com"
        ),
        "{effects:?}"
    );
    assert!(drawn(&effects).is_none(), "nothing to draw yet");
    assert_eq!(focus.key_context(), KeyContext::ContactPopover);
}

#[test]
fn the_answer_is_drawn_with_the_person_their_counts_and_what_is_open() {
    let mut focus = ready();
    let view = open(&mut focus, None);
    assert_eq!(view.name, "Ada Moreno");
    assert_eq!(view.role, "Finance lead, Northwind Partners");
    assert_eq!(view.tiles[0].value, "431");
    assert_eq!(view.open[0].label, "Waiting on you");
    assert_eq!(on(&view), ["Inbox"]);
    assert_eq!(focus.key_context(), KeyContext::ContactPopover);
}

#[test]
fn a_click_on_a_name_asks_about_that_address_whatever_row_the_cursor_is_on() {
    let mut focus = ready();
    let effects = focus.handle_on(
        Input::ContactPopoverAt("grace@example.com".to_owned()),
        &List,
    );
    assert!(
        matches!(
            &asks(&effects)[..],
            [(_, Request::ContactGlance { address, .. })] if address == "grace@example.com"
        ),
        "{effects:?}"
    );
}

#[test]
fn the_arrows_change_where_mail_arrives_at_once_and_stop_at_the_ends() {
    let mut focus = ready();
    let _ = open(&mut focus, None);

    let effects = run(&mut focus, CommandId::ContactArrivalNext);
    assert!(
        matches!(
            &asks(&effects)[..],
            [(_, Request::Post(Command::ContactSetArrival { address, arrival }))]
                if address == "ada@example.com" && *arrival == Some(ContactArrival::Digest)
        ),
        "the change is a command: {effects:?}"
    );
    assert_eq!(on(&drawn(&effects).expect("redrawn at once")), ["Digest"]);

    let effects = run(&mut focus, CommandId::ContactArrivalNext);
    assert_eq!(on(&drawn(&effects).unwrap()), ["Filtered out"]);
    // The end of the line: nothing changes, nothing is sent.
    let effects = run(&mut focus, CommandId::ContactArrivalNext);
    assert!(asks(&effects).is_empty(), "{effects:?}");

    let effects = run(&mut focus, CommandId::ContactArrivalPrev);
    assert_eq!(on(&drawn(&effects).unwrap()), ["Digest"]);
}

#[test]
fn a_click_on_a_segment_does_what_the_arrow_does() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    let effects = focus.handle_on(Input::ContactArrival(ContactArrival::Filtered), &List);
    assert!(matches!(
        &asks(&effects)[..],
        [(_, Request::Post(Command::ContactSetArrival { arrival, .. }))]
            if *arrival == Some(ContactArrival::Filtered)
    ));
    assert_eq!(on(&drawn(&effects).unwrap()), ["Filtered out"]);
}

#[test]
fn zero_goes_back_to_the_default_rules() {
    let mut focus = ready();
    let _ = open(&mut focus, Some(ContactArrival::Digest));
    let effects = run(&mut focus, CommandId::ContactSetArrival);
    assert!(matches!(
        &asks(&effects)[..],
        [(_, Request::Post(Command::ContactSetArrival { address, arrival: None }))]
            if address == "ada@example.com"
    ));
    assert_eq!(on(&drawn(&effects).unwrap()), ["Inbox"]);
}

#[test]
fn return_is_all_their_mail_through_the_search_it_would_type() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    let effects = run(&mut focus, CommandId::ContactShowMail);
    let intents = shown(&effects);
    assert!(
        intents.contains(&Intent::CloseSurface(SurfaceKind::ContactPopover)),
        "{intents:?}"
    );
    assert!(
        intents.iter().any(|intent| matches!(
            intent,
            Intent::OpenBar { text, .. } if text == "with:ada@example.com"
        )),
        "{intents:?}"
    );
}

#[test]
fn c_writes_to_them_and_the_popover_goes() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    let effects = run(&mut focus, CommandId::ContactCompose);
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::WriteTo("ada@example.com".to_owned())));
    assert!(intents.contains(&Intent::CloseSurface(SurfaceKind::ContactPopover)));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn escape_puts_it_away_and_the_keyboard_goes_home() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    let effects = run(&mut focus, CommandId::Back);
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::CloseSurface(SurfaceKind::ContactPopover)));
    assert!(intents.contains(&Intent::KeyboardHome));
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn other_keys_do_nothing_under_the_popover() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    for command in [CommandId::Archive, CommandId::NextMessage] {
        let effects = run(&mut focus, command);
        assert!(
            shown(&effects).is_empty() && asks(&effects).is_empty(),
            "{command:?}: {effects:?}"
        );
        assert_eq!(focus.key_context(), KeyContext::ContactPopover);
    }
}

#[test]
fn nobody_at_that_address_means_no_popover() {
    let mut focus = ready();
    let effects = run(&mut focus, CommandId::ContactPopover);
    let [(ticket, Request::ContactGlance { stamp, .. })] = asks(&effects)[..] else {
        panic!("asked");
    };
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactGlance {
                stamp,
                glance: Ok(None),
            },
        ),
        &List,
    );
    assert!(drawn(&effects).is_none());
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn an_answer_for_a_popover_already_put_away_is_dropped() {
    let mut focus = ready();
    let effects = run(&mut focus, CommandId::ContactPopover);
    let [(ticket, Request::ContactGlance { stamp, .. })] = asks(&effects)[..] else {
        panic!("asked");
    };
    let _ = run(&mut focus, CommandId::Back);
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactGlance {
                stamp,
                glance: Ok(Some(Box::new(ada(None)))),
            },
        ),
        &List,
    );
    assert!(drawn(&effects).is_none(), "{effects:?}");
    assert_eq!(focus.key_context(), KeyContext::List);
}

#[test]
fn a_click_away_forgets_it_without_a_close_echo() {
    let mut focus = ready();
    let _ = open(&mut focus, None);
    let _ = focus.handle_on(Input::SurfaceClosed(SurfaceKind::ContactPopover), &List);
    assert_eq!(focus.key_context(), KeyContext::List);
    let effects = run(&mut focus, CommandId::ContactArrivalNext);
    assert!(asks(&effects).is_empty(), "nothing is open to change");
}

#[test]
fn a_sender_the_default_rules_filter_starts_on_filtered_out_and_the_arrow_goes_from_there() {
    let mut focus = ready();
    let effects = run(&mut focus, CommandId::ContactPopover);
    let [(ticket, Request::ContactGlance { stamp, .. })] = asks(&effects)[..] else {
        panic!("asked");
    };
    let mut glance = ada(None);
    glance.default_arrival = ContactArrival::Filtered;
    let effects = focus.handle_on(
        Input::Reply(
            ticket,
            Reply::ContactGlance {
                stamp,
                glance: Ok(Some(Box::new(glance))),
            },
        ),
        &List,
    );
    assert_eq!(on(&drawn(&effects).unwrap()), ["Filtered out"]);
    let effects = run(&mut focus, CommandId::ContactArrivalPrev);
    assert_eq!(on(&drawn(&effects).unwrap()), ["Digest"]);
}

#[test]
fn open_card_is_enabled_and_opens_the_contacts_view_on_the_person() {
    let mut focus = ready();
    let view = open(&mut focus, None);
    let card = view
        .actions
        .iter()
        .find(|action| action.label == "Open card")
        .unwrap();
    assert!(card.enabled, "the Contacts view exists on the Mac");
    let effects = run(&mut focus, CommandId::ContactOpenCard);
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::CloseSurface(SurfaceKind::ContactPopover)));
    assert_eq!(focus.key_context(), KeyContext::Contacts);
    let asked: Vec<Request> = asks(&effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect();
    assert!(asked.contains(&Request::ContactsOverview));
    assert!(
        asked.iter().any(|request| matches!(
            request,
            Request::ContactsFiltered { text, .. } if text == "ada@example.com"
        )),
        "the list is found down to them: {asked:?}"
    );
}
