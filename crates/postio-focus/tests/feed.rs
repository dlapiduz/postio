//! The feed's rules, from the controller's side (specs/009-focus-macos T023).
//!
//! What GTK's `list/feed.rs` did, asserted here so both frontends get it: a
//! place opened is counted before the list changes over, an answer for a
//! place no longer in view changes nothing, a page is read with its pills and
//! delivered under the list's own stamp, a failed page is asked again a
//! bounded number of times, and an engine event re-reads what it moved.

use postio_config::paths::Platform;
use postio_core::Event;
use postio_focus::{
    Effect, FocusController, Input, Intent, Opened, PageAnswer, Policy, Reply, Request, Ticket,
};
use postio_model::listing::PageRequest;
use postio_model::{AccountId, FocusScope, ListScope, MailboxId, MessageId};

const INBOX: ListScope = ListScope::Focus(FocusScope::Inbox);
const HAS_ACTION: ListScope = ListScope::Focus(FocusScope::HasAction);

fn focus() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

/// The one ask among `effects`.
fn ask(effects: &[Effect]) -> (Ticket, Request) {
    let asks: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(asks.len(), 1, "one ask in {effects:?}");
    asks[0].clone()
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

fn opened(scope: ListScope, total: u32) -> Opened {
    Opened {
        scope,
        folders: vec![(MailboxId::new(1), true), (MailboxId::new(2), false)],
        total: Ok(total),
        surfaced: (scope == INBOX).then(|| Ok(Vec::new())),
    }
}

/// `focus` with `scope` open and counted at `total`.
fn open_at(focus: &mut FocusController, scope: ListScope, total: u32) {
    let (ticket, _) = ask(&focus.open(scope));
    focus.handle(Input::Reply(ticket, Reply::Opened(opened(scope, total))));
}

#[test]
fn opening_a_place_counts_it_and_reads_what_it_splices() {
    let mut focus = focus();
    let (_, request) = ask(&focus.open(INBOX));
    assert_eq!(
        request,
        Request::OpenScope {
            scope: INBOX,
            splices: true
        }
    );
    let (_, request) = ask(&focus.open(HAS_ACTION));
    assert_eq!(
        request,
        Request::OpenScope {
            scope: HAS_ACTION,
            splices: false
        },
        "only Focus's own inbox has rows spliced in"
    );
}

#[test]
fn the_list_changes_over_once_the_place_is_counted() {
    let mut focus = focus();
    let (ticket, _) = ask(&focus.open(INBOX));
    let effects = focus.handle(Input::Reply(ticket, Reply::Opened(opened(INBOX, 120))));
    assert_eq!(shown(&effects), vec![Intent::ReplaceSource { total: 120 }]);
    assert_eq!(focus.list_total(), 120);
    assert!(focus.is_inbox(MailboxId::new(1)));
    assert!(!focus.is_inbox(MailboxId::new(2)));
}

#[test]
fn a_count_for_a_place_left_behind_changes_nothing() {
    let mut focus = focus();
    let (first, _) = ask(&focus.open(INBOX));
    let _ = focus.open(HAS_ACTION);
    assert!(
        focus
            .handle(Input::Reply(first, Reply::Opened(opened(INBOX, 120))))
            .is_empty()
    );
    assert_eq!(focus.list_total(), 0);
}

#[test]
fn a_page_is_asked_for_under_the_lists_stamp_and_delivered_with_it() {
    let mut focus = focus();
    open_at(&mut focus, HAS_ACTION, 120);
    let effects = focus.page_wanted(1, 7);
    assert_eq!(
        shown(&effects),
        vec![Intent::PagePending { stamp: 7, page: 1 }]
    );
    let (ticket, request) = ask(&effects);
    let Request::Page {
        page,
        stamp,
        start,
        count,
        wanted,
    } = request
    else {
        panic!("a page read, not {request:?}");
    };
    assert_eq!((page, stamp, start, count), (1, 7, 50, 50));
    assert_eq!(
        wanted,
        PageRequest {
            scope: HAS_ACTION,
            offset: 50,
            limit: 50
        }
    );
    assert_eq!(focus.pages_asked(), &[1]);

    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::Page {
            page,
            stamp,
            start,
            count,
            answer: Ok(PageAnswer::Threads {
                total: 120,
                rows: Vec::new(),
                labels: Vec::new(),
            }),
        },
    ));
    assert_eq!(
        shown(&effects),
        vec![
            Intent::DeliverPage {
                stamp: 7,
                page: 1,
                total: 120,
                rows: Vec::new()
            },
            Intent::Filled
        ]
    );
    assert!(focus.has_landed());
    assert!(focus.take_opened(), "the cursor goes to the new list once");
    assert!(!focus.take_opened(), "and a re-read leaves it where it is");
}

#[test]
fn a_failed_page_is_asked_again_a_bounded_number_of_times() {
    let mut focus = focus();
    open_at(&mut focus, HAS_ACTION, 10);
    let (mut ticket, mut request) = ask(&focus.page_wanted(0, 3));
    let mut asked = 1;
    loop {
        let Request::Page {
            page,
            stamp,
            start,
            count,
            ..
        } = request
        else {
            panic!("a page read");
        };
        let effects = focus.handle(Input::Reply(
            ticket,
            Reply::Page {
                page,
                stamp,
                start,
                count,
                answer: Err("the store is busy".to_owned()),
            },
        ));
        if shown(&effects).contains(&Intent::GiveUp { stamp: 3, page: 0 }) {
            break;
        }
        assert!(shown(&effects).contains(&Intent::AbandonPage { stamp: 3, page: 0 }));
        (ticket, request) = ask(&effects);
        asked += 1;
        assert!(asked < 10, "a failing store is not asked for ever");
    }
    assert!(asked > 1, "a failure is asked again before giving up");
}

#[test]
fn mail_leaving_is_said_before_the_list_rereads() {
    let mut focus = focus();
    open_at(&mut focus, HAS_ACTION, 10);
    let effects = focus.handle(Input::Event(Event::MessagesRemoved {
        account: AccountId::new(1),
        mailbox: MailboxId::new(1),
        messages: vec![MessageId::new(5)],
    }));
    let (_, request) = ask(&effects);
    assert_eq!(
        request,
        Request::NoteRemoved {
            mailbox: MailboxId::new(1),
            messages: vec![MessageId::new(5)]
        }
    );
}

#[test]
fn in_the_inbox_an_event_rereads_what_is_surfaced_first() {
    let mut focus = focus();
    open_at(&mut focus, INBOX, 10);
    let effects = focus.handle(Input::Event(Event::SurfacedChanged));
    let (ticket, request) = ask(&effects);
    assert_eq!(request, Request::Surfaced);
    assert!(
        shown(&effects).is_empty(),
        "nothing re-read before it lands"
    );
    let effects = focus.handle(Input::Reply(ticket, Reply::Surfaced(Ok(Vec::new()))));
    assert_eq!(shown(&effects), vec![Intent::RefreshList]);
}

#[test]
fn elsewhere_mail_arriving_in_an_inbox_rereads_the_list() {
    let mut focus = focus();
    open_at(&mut focus, HAS_ACTION, 10);
    let effects = focus.handle(Input::Event(Event::MessageListChanged {
        account: AccountId::new(1),
        mailbox: MailboxId::new(1),
    }));
    assert_eq!(shown(&effects), vec![Intent::RefreshList]);
}
